-- Set by the transitions to LIVE and ENDED.
ALTER TABLE broadcasts
    ADD COLUMN started_at TIMESTAMPTZ NULL,
    ADD COLUMN ended_at   TIMESTAMPTZ NULL;
