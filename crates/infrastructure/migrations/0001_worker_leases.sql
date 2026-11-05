CREATE TABLE worker_leases (
    worker_id  SMALLINT PRIMARY KEY CHECK (worker_id BETWEEN 0 AND 1023),
    holder     TEXT,
    expires_at TIMESTAMPTZ NOT NULL DEFAULT '-infinity',
    last_ms    BIGINT NOT NULL DEFAULT 0 CHECK (last_ms >= 0)
);

INSERT INTO worker_leases (worker_id) SELECT generate_series(0, 1023);
