# Politique de sécurité

## Portée

`tessera-core` est un moteur multijoueur **self-host** : chaque opérateur fait tourner
son propre serveur et gère ses propres comptes/accès. Il n'y a pas d'infrastructure
centrale exploitée par ce projet. Sont dans le périmètre de cette politique :

- `server/` (Gateway + Shards, transport réseau, désérialisation des messages clients)
- `protocol/` (schémas FlatBuffers, contrat réseau)
- `directory/` (signature Ed25519 du `servers.json`)
- `client-mod/` (mod RED4ext/redscript côté jeu)
- `voip/` (intégration Mumble)

Le serveur reçoit et désérialise des paquets réseau non fiables (clients) : les failles
de type déni de service, dépassement de tampon, désérialisation non sûre ou usurpation
d'identité de shard/gateway sont particulièrement pertinentes ici.

## Signaler une vulnérabilité

**Ne pas ouvrir d'issue publique.** Utilise l'onglet
[Security](https://github.com/The-Genium007/tessera-core/security/advisories/new) du
dépôt GitHub pour ouvrir une **security advisory privée** — c'est le canal de
signalement.

Merci d'inclure : la version/commit concerné, les étapes de reproduction, et l'impact
estimé (accès non autorisé, crash, corruption d'état, etc.).

## Versions couvertes

Le projet est pré-1.0 (versionnement `0.x`) : seule la dernière version publiée sur la
branche `main` (canal `stable`) reçoit des correctifs de sécurité.
