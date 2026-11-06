-- The title check trims every character Rust's `char::is_whitespace` matches,
-- not just spaces as plain `btrim` would, so a row passes here exactly when
-- `BroadcastTitle::parse` accepts it on the way back out.
CREATE TABLE broadcasts (
    id          BIGINT PRIMARY KEY,
    owner_id    BIGINT NOT NULL REFERENCES users (id),
    title       TEXT NOT NULL CHECK (
        char_length(btrim(title, U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'))
            BETWEEN 1 AND 100
    ),
    description TEXT NOT NULL DEFAULT '' CHECK (char_length(description) <= 2000),
    status      TEXT NOT NULL CHECK (status IN ('READY', 'LIVE', 'INTERRUPTED', 'ENDED')),
    version     INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ NULL
);

CREATE INDEX broadcasts_live_list_idx ON broadcasts (id DESC) WHERE deleted_at IS NULL;
CREATE INDEX broadcasts_owner_id_idx ON broadcasts (owner_id);
