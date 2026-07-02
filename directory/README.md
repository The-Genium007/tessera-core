# directory/ — Dérivation et signature du servers.json (tessera-directory)

Outil (`tessera-directory`) qui dérive le `servers.json` public — consommé par le navigateur de
serveurs du launcher — à partir du **manifeste serveur** d'un opérateur (`server::manifest`,
voir `server/server.example.toml`), et le signe (Ed25519 détaché, clé dédiée au directory,
distincte de celle des modsets). Modèle self-host : chaque opérateur publie son propre
`servers.json` signé, pas d'annuaire central.

Sous-commandes : `publish` (dérive+signe), `verify` (vérifie une paire fichier/signature),
`topology check`/`topology render` (valide/visualise la géométrie de la topologie de shards
avant déploiement).

Voir `docs/superpowers/specs/2026-07-02-m6-server-manifest-directory-design.md`.

**Statut :** en cours d'implémentation (M6 / chantier A).
**Plateforme :** macOS/Linux/Windows.
