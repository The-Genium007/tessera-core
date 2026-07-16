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

## Attestation officielle (kind=official)

Un manifeste qui déclare `identity.kind = "official"` n'est republié `official` dans
`servers.json` que si `publish` peut **prouver** l'attestation à l'exécution ; sinon il retombe
silencieusement sur `community` (jamais bloquant). `publish` n'est **pas** un service : c'est un
outil ponctuel invoqué à la demande (`docker run --rm --entrypoint tessera-directory <image> publish …`,
cf. `tessera-core/server/docker-compose.yml`). L'environnement où il tourne doit exposer 4 variables :

- `TESSERA_INTERNAL_ATTESTATION_URL` — endpoint interne du serveur qui rend son token d'attestation courant.
- `TESSERA_ZITADEL_JWKS_URI` — clés publiques ZITADEL pour vérifier la signature du token.
- `TESSERA_ZITADEL_ISSUER` — émetteur attendu dans le token.
- `TESSERA_CMS_URL` — CMS interrogé pour confirmer que le `sub` du token = une entrée `officialServers` connue.

Ces 4 variables sont documentées en détail (source de la valeur, pièges) dans
`tessera-core/server/.env.example` — s'y référer plutôt que de dupliquer ici. Le flux opérateur
complet (provisionnement du Service User ZITADEL par le CMS, injection de
`ZITADEL_SERVICE_ACCOUNT_KEY_JSON` côté serveur de jeu, séquence de bout en bout) est décrit dans
`docs/superpowers/specs/2026-07-15-official-server-zitadel-attestation-design.md` (« Vue d'ensemble du flux »).

**Statut :** en cours d'implémentation (M6 / chantier A).
**Plateforme :** macOS/Linux/Windows.
