#!/usr/bin/env bash
# backup-players.sh — sauvegarde v0 du store joueurs (chantier 4.6, audit prod 2026-07-03).
#
# Temporaire : remplacé par un backup pg_dump quand la persistance passe sur Postgres
# (palier 1, §5.2 de docs/superpowers/specs/2026-07-03-production-roadmap-design.md).
# En attendant, « aucun backup » n'est pas une option acceptable.
#
# Copie players.json depuis le volume nommé du service `gateway` (écriture atomique
# tmp+rename côté serveur, cf. persistence.rs — copier le fichier pendant qu'il tourne est
# sûr) et le pousse, compressé et horodaté, vers un object storage S3-compatible (OVH).
#
# Prérequis : AWS CLI installé (`aws`), configuré pour parler à un endpoint S3-compatible.
# Rétention : gérée par une règle de lifecycle sur le bucket (pas par ce script) — configurer
# une fois sur le bucket OVH : 7 jours (quotidien) / ~28 jours (hebdo) / ~180 jours (mensuel).
#
# Variables d'environnement requises :
#   BACKUP_S3_ENDPOINT   — ex. https://s3.gra.io.cloud.ovh.net
#   BACKUP_S3_BUCKET     — ex. tessera-backups
#   AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY — identifiants OVH Object Storage (S3 API)
#
# Variables optionnelles :
#   GATEWAY_CONTAINER    — nom du conteneur Gateway (défaut : server-gateway-1, nom généré
#                          par `docker compose` pour le service `gateway` de ce répertoire)
#
# Installation (cron quotidien, sur le VPS, dans le répertoire de ce compose) :
#   0 3 * * * BACKUP_S3_ENDPOINT=... BACKUP_S3_BUCKET=... AWS_ACCESS_KEY_ID=... \
#     AWS_SECRET_ACCESS_KEY=... /path/to/backup-players.sh >> /var/log/tessera-backup.log 2>&1
#
# Restauration (manuelle, à tester au moins une fois — cf. runbook §7 de la spec) :
#   aws --endpoint-url "$BACKUP_S3_ENDPOINT" s3 cp s3://$BACKUP_S3_BUCKET/<fichier>.json.gz - \
#     | gunzip > players.json
#   docker cp players.json <conteneur-gateway>:/data/players.json
#   docker compose restart gateway

set -euo pipefail

: "${BACKUP_S3_ENDPOINT:?BACKUP_S3_ENDPOINT non défini}"
: "${BACKUP_S3_BUCKET:?BACKUP_S3_BUCKET non défini}"
: "${AWS_ACCESS_KEY_ID:?AWS_ACCESS_KEY_ID non défini}"
: "${AWS_SECRET_ACCESS_KEY:?AWS_SECRET_ACCESS_KEY non défini}"

GATEWAY_CONTAINER="${GATEWAY_CONTAINER:-server-gateway-1}"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

RAW_FILE="$TMP_DIR/players.json"
GZ_FILE="$TMP_DIR/players-$TIMESTAMP.json.gz"

echo "[backup-players] copie de /data/players.json depuis $GATEWAY_CONTAINER"
docker cp "$GATEWAY_CONTAINER:/data/players.json" "$RAW_FILE"

gzip -c "$RAW_FILE" > "$GZ_FILE"

DEST="s3://$BACKUP_S3_BUCKET/players-$TIMESTAMP.json.gz"
echo "[backup-players] envoi vers $DEST"
aws --endpoint-url "$BACKUP_S3_ENDPOINT" s3 cp "$GZ_FILE" "$DEST"

echo "[backup-players] OK — $DEST"
