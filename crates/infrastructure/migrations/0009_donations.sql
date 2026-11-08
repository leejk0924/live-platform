-- One row per donation order. The id is also the order id the payment
-- provider sees. An order is credited exactly once: only a PENDING row
-- becomes DONE, and only together with the payment that paid it. The amount
-- check mirrors domain::donation::DonationAmount.
CREATE TABLE donations (
    id           BIGINT PRIMARY KEY,
    broadcast_id BIGINT NOT NULL REFERENCES broadcasts (id),
    donor_id     BIGINT NOT NULL REFERENCES users (id),
    amount       BIGINT NOT NULL CHECK (amount BETWEEN 1000 AND 1000000),
    status       TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'DONE')),
    -- UNIQUE: one payment can never pay for two orders.
    payment_key  TEXT NULL UNIQUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    approved_at  TIMESTAMPTZ NULL,
    CHECK ((status = 'DONE') = (payment_key IS NOT NULL AND approved_at IS NOT NULL))
);

CREATE INDEX donations_broadcast_done ON donations (broadcast_id, id DESC)
    WHERE status = 'DONE';

-- Kept in the same transaction as every credit, so a broadcast's total is
-- one row read. Its row lock also orders one broadcast's credits, so the
-- totals they announce only grow.
CREATE TABLE donation_totals (
    broadcast_id BIGINT PRIMARY KEY REFERENCES broadcasts (id),
    total        BIGINT NOT NULL CHECK (total >= 0),
    count        BIGINT NOT NULL CHECK (count >= 0)
);
