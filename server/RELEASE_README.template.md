# Tessera Server — bundle self-host

Ce zip contient tout le nécessaire pour héberger un serveur Tessera :

- `docker-compose.yml` — topologie Gateway + 2 Shards + heartbeat (voir les commentaires du
  fichier pour le détail des ports/volumes).
- `server.docker.toml` — configuration du serveur. **Le champ `required_modset` est déjà
  renseigné** avec la version de modset client compatible avec ce serveur — ne pas l'éditer à la
  main, il est synchronisé automatiquement à chaque publication.

## Démarrage

```bash
docker compose up -d
```

Ouvrir le port UDP du Gateway (27020 par défaut) sur le pare-feu — le protocole de jeu n'est pas
HTTP, pas de tunnel Cloudflare possible pour ce port.

## Mise à jour

Retélécharger le bundle de la version souhaitée (dev/playtest/main) et remplacer
`docker-compose.yml`/`server.docker.toml` par les nouveaux — l'image Docker référencée est tirée
automatiquement (`pull_policy: always`).
