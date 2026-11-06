CREATE TABLE broadcasts (
    id          BIGINT PRIMARY KEY,
    owner_id    BIGINT NOT NULL REFERENCES users (id),
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL CHECK (status IN ('READY', 'LIVE', 'INTERRUPTED', 'ENDED')),
    version     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ NULL
);

CREATE INDEX broadcasts_live_list_idx ON broadcasts (id DESC) WHERE deleted_at IS NULL;
CREATE INDEX broadcasts_owner_id_idx ON broadcasts (owner_id);
