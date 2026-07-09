-- 0001_create_player_records.sql
-- Domaine "identité/progression" (règle wipe-ready, schéma par domaines, R9 spec roadmap).
-- Clé = subject OIDC pour un compte vérifié (serveurs identity.public=true) ; pour un serveur
-- privé, `display_name` reste utilisé via FileStore (Postgres n'est pas requis en mode privé).
CREATE TABLE player_records (
    subject         TEXT PRIMARY KEY,       -- sub OIDC ZITADEL
    display_name    TEXT NOT NULL,          -- pseudo lié au compte (premier arrivé, premier servi)
    last_position   REAL[3] NOT NULL,
    residence       REAL[3],
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_player_records_display_name ON player_records (display_name);
