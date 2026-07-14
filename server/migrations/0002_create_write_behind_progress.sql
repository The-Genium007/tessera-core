-- tessera-core/server/migrations/0002_create_write_behind_progress.sql
-- Marque haute du mécanisme write-behind (design 2026-07-14, données partagées) : dernier
-- numéro de séquence de journal local appliqué avec succès, par flux. `stream_id` permet
-- plusieurs journaux indépendants plus tard (un futur domaine par flux) ; un seul flux utilisé
-- pour l'instant, aucun domaine économie/inventaire n'existe encore.
CREATE TABLE write_behind_progress (
    stream_id        TEXT PRIMARY KEY,
    last_applied_seq BIGINT NOT NULL
);
