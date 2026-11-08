-- One finished MediaMTX segment of a broadcast. `source` is its path in the
-- spool, so a hook delivered twice registers it once. `retry_at` is both the
-- claim lease and the backoff: a pending row is due once it has passed.
-- Status values mirror domain::recording::RecordingStatus.
CREATE TABLE recordings (
    id           BIGINT PRIMARY KEY,
    broadcast_id BIGINT NOT NULL REFERENCES broadcasts (id),
    source       TEXT NOT NULL UNIQUE,
    storage_key  TEXT NOT NULL UNIQUE,
    status       TEXT NOT NULL CHECK (status IN ('PENDING', 'READY', 'FAILED')),
    duration_ms  BIGINT NOT NULL CHECK (duration_ms >= 0),
    size_bytes   BIGINT NULL CHECK (size_bytes >= 0),
    attempts     INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    retry_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    recorded_at  TIMESTAMPTZ NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((status = 'READY') = (size_bytes IS NOT NULL))
);

-- The public list walks the primary key backwards; one broadcast's list
-- needs its own.
CREATE INDEX recordings_broadcast_newest ON recordings (broadcast_id, id DESC);
CREATE INDEX recordings_pending_due ON recordings (retry_at) WHERE status = 'PENDING';
