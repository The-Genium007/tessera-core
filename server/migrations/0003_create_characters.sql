-- Table "personnages" — modèle multi (owner, character_id), remplace conceptuellement
-- player_records (conservée pour la migration des données existantes, cf. Task 3). owner = clé
-- de compte (display_name ou subject OIDC, jamais montré aux autres joueurs). pseudonym =
-- identité RP visible, UNIQUE sur tout le serveur (contrainte globale, pas par owner).
CREATE TABLE characters (
    id            BIGSERIAL PRIMARY KEY,
    owner         TEXT NOT NULL,
    pseudonym     TEXT NOT NULL,
    appearance    BYTEA NOT NULL DEFAULT '',
    last_position REAL[3] NOT NULL,
    residence     REAL[3],
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_characters_pseudonym ON characters (pseudonym);
CREATE INDEX idx_characters_owner ON characters (owner);
