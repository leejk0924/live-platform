-- Each publish token lets one publisher in. A row is the token's use; the
-- primary key makes a second use a conflict. Rows past expires_at are
-- useless (the token is refused anyway) and the reaper deletes them.
CREATE TABLE publish_token_uses (
    jti          TEXT PRIMARY KEY CHECK (char_length(jti) BETWEEN 1 AND 64),
    broadcast_id BIGINT NOT NULL REFERENCES broadcasts (id),
    expires_at   TIMESTAMPTZ NOT NULL,
    used_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX publish_token_uses_expires_at_idx ON publish_token_uses (expires_at);
