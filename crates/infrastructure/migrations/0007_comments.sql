-- Every write to a comment (post, edit, delete) takes the next value, so a
-- broadcast's change feed is one index range: "revision > R" returns every
-- change after R, deleted comments included as tombstones. Writes to one
-- broadcast hold an advisory lock until they commit (comment_write_lock in
-- the adapter), so its revisions become visible in ascending order.
CREATE SEQUENCE comment_revisions;

-- The body check mirrors domain::comment::CommentBody (1 to 500 characters);
-- the domain also trims and refuses control characters before writing.
CREATE TABLE comments (
    id           BIGINT PRIMARY KEY,
    broadcast_id BIGINT NOT NULL REFERENCES broadcasts (id),
    author_id    BIGINT NOT NULL REFERENCES users (id),
    body         TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 500),
    version      INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    revision     BIGINT NOT NULL DEFAULT nextval('comment_revisions'),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NULL,
    deleted_at   TIMESTAMPTZ NULL
);

ALTER SEQUENCE comment_revisions OWNED BY comments.revision;

CREATE INDEX comments_broadcast_newest ON comments (broadcast_id, id DESC)
    WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX comments_broadcast_revision ON comments (broadcast_id, revision);

-- Slow mode in seconds, 0 = off; domain::comment::SLOW_MODE_MAX is the upper
-- bound. Not part of the broadcast's version: toggling it must not conflict
-- with an open edit of the title.
ALTER TABLE broadcasts ADD COLUMN slow_mode_seconds INTEGER NOT NULL DEFAULT 0
    CHECK (slow_mode_seconds BETWEEN 0 AND 300);
