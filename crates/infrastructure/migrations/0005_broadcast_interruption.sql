-- When the publisher left, kept only while INTERRUPTED; the reaper ends the
-- broadcast once it is older than domain::broadcast::INTERRUPT_GRACE.
ALTER TABLE broadcasts ADD COLUMN interrupted_at TIMESTAMPTZ NULL;

UPDATE broadcasts SET interrupted_at = updated_at WHERE status = 'INTERRUPTED';

ALTER TABLE broadcasts ADD CONSTRAINT broadcasts_interrupted_at_check
    CHECK ((status = 'INTERRUPTED') = (interrupted_at IS NOT NULL));

-- Before the index below, an owner could hold several live broadcasts. Keep
-- the newest one on air and end the rest, so the index can be built. The
-- reaper disconnects any publisher still sending to one of those.
UPDATE broadcasts b
SET status = 'ENDED', ended_at = now(), interrupted_at = NULL,
    version = version + 1, updated_at = now()
WHERE b.status IN ('LIVE', 'INTERRUPTED') AND b.deleted_at IS NULL
  AND EXISTS (
      SELECT 1 FROM broadcasts newer
      WHERE newer.owner_id = b.owner_id AND newer.id > b.id
        AND newer.status IN ('LIVE', 'INTERRUPTED') AND newer.deleted_at IS NULL
  );

-- When the broadcast last went LIVE, kept only while LIVE. The reaper
-- interrupts a LIVE broadcast with no stream once this is older than the
-- grace; an edit to the title does not move it.
ALTER TABLE broadcasts ADD COLUMN live_since TIMESTAMPTZ NULL;

UPDATE broadcasts SET live_since = updated_at WHERE status = 'LIVE';

ALTER TABLE broadcasts ADD CONSTRAINT broadcasts_live_since_check
    CHECK ((status = 'LIVE') = (live_since IS NOT NULL));

-- One broadcast on air per owner. The status list is
-- domain::broadcast::BroadcastStatus::ACTIVE; a test keeps them equal, and the
-- adapter maps a violation of this index by name.
CREATE UNIQUE INDEX broadcasts_one_active_per_owner ON broadcasts (owner_id)
    WHERE status IN ('LIVE', 'INTERRUPTED') AND deleted_at IS NULL;
