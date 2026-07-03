# Contribuer à tessera-core

Merci de contribuer. Ce document décrit les conventions du dépôt — courtes et strictes,
pour que les revues restent rapides.

## Prérequis

- Rust stable (le workspace se build avec `cargo build` depuis la racine de `tessera-core/`).
- Aucune dépendance native n'est nécessaire pour compiler et tester par défaut ; la
  feature `gns` (transport réseau réel) exige protobuf 3.21.x et cmake — voir
  [`docs/0003-gns-binding.md`](docs/0003-gns-binding.md).
- Le client-mod (`client-mod/`) se build et se teste sur **Windows uniquement**
  (MSVC, vcpkg, CMake) avec le jeu en version **2.31** — voir
  [`docs/0004-windows-modding-toolchain.md`](docs/0004-windows-modding-toolchain.md).

## Workflow

1. **TDD côté Rust** : test rouge → implémentation minimale → test vert → commit.
   Toute nouvelle logique serveur/protocole/directory arrive avec ses tests.
   Côté client-mod (in-game), les tests automatisés sont impossibles : documente la
   vérification manuelle effectuée dans la description de la PR.
2. **Un fichier = une responsabilité.** Préfère des fichiers focalisés à des modules
   fourre-tout.
3. Avant de pousser :

   ```bash
   cargo test
   cargo fmt && cargo clippy
   ```

   `clippy` doit passer sans warning sur le code modifié.

## Conventions de commits

[Conventional Commits](https://www.conventionalcommits.org/) :

- `feat:` nouvelle fonctionnalité
- `fix:` correction de bug
- `docs:` documentation uniquement
- `chore:` maintenance (deps, CI, tooling)
- `refactor:` refactoring sans changement de comportement
- `test:` ajout/modification de tests

Exemple : `feat(server): handoff des joueurs entre shards adjacents`.

## Décisions d'architecture

Les décisions notables sont documentées en ADR dans [`docs/`](docs/). Si ta
contribution change une décision d'architecture (transport, protocole, sharding,
version de jeu cible...), ouvre d'abord une issue pour en discuter ; un ADR nouveau ou
amendé accompagnera le changement.

## Assets CD Projekt Red — règle absolue

**Ce dépôt ne redistribue aucun asset de CD Projekt Red.** Le moteur fonctionne en
**modding runtime uniquement** : le mod se charge dans une installation légitime du jeu,
et aucun fichier de jeu extrait, converti ou redistribué n'entre dans ce dépôt.
Toute PR contenant des assets, données ou binaires issus du jeu sera refusée.

## Licence

Licence : à déterminer. En contribuant, tu acceptes que ta contribution soit publiée
sous la licence qui sera retenue pour le projet.
