CREATE TABLE bans (
    id           BIGSERIAL PRIMARY KEY,
    subject      TEXT,
    ip           TEXT,
    hwid_hash    TEXT,
    scope        TEXT NOT NULL, -- 'temp' | 'perm'
    reason       TEXT NOT NULL,
    expires_at   TIMESTAMPTZ,   -- NULL = permanent
    banned_by    TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT bans_at_least_one_vector CHECK (
        subject IS NOT NULL OR ip IS NOT NULL OR hwid_hash IS NOT NULL
    )
);
CREATE INDEX idx_bans_subject ON bans (subject) WHERE subject IS NOT NULL;
CREATE INDEX idx_bans_ip ON bans (ip) WHERE ip IS NOT NULL;
CREATE INDEX idx_bans_hwid_hash ON bans (hwid_hash) WHERE hwid_hash IS NOT NULL;
