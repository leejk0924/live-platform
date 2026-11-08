use std::time::SystemTime;

use application::ports::{
    PortError,
    donation::{
        DonationCredit, DonationOrder, DonationQuery, DonationRepository, DonationSummary,
        DonationView, NewDonation, Payment, Settlement,
    },
    event_bus::{EventKind, RealtimeEvent},
};
use async_trait::async_trait;
use domain::{
    auth::Username,
    broadcast::{BroadcastId, BroadcastStatus},
    donation::{DonationAmount, DonationId, DonationStatus, PaymentKey, accepts_donations},
    id::SnowflakeId,
};
use sqlx::{PgPool, types::time::OffsetDateTime};

use super::status_names;
use crate::realtime;

fn port_error(err: sqlx::Error) -> PortError {
    PortError(err.to_string())
}

const UNIQUE_VIOLATION: &str = "23505";

fn invalid(what: &str, id: i64) -> PortError {
    PortError(format!("invalid stored {what} for donation {id}"))
}

fn snowflake(raw: i64, what: &str, id: i64) -> Result<SnowflakeId, PortError> {
    SnowflakeId::try_from(raw).map_err(|_| invalid(what, id))
}

/// `d.id, d.broadcast_id, d.amount, d.approved_at, u.username` of a
/// credited donation.
type ViewRow = (i64, i64, i64, OffsetDateTime, String);

fn to_view(
    (id, broadcast_id, amount, approved_at, donor_name): ViewRow,
) -> Result<DonationView, PortError> {
    Ok(DonationView {
        id: snowflake(id, "id", id)?,
        broadcast_id: snowflake(broadcast_id, "broadcast_id", id)?,
        donor_name: Username::parse(&donor_name).map_err(|_| invalid("donor name", id))?,
        amount: DonationAmount::parse(amount).map_err(|_| invalid("amount", id))?,
        approved_at: SystemTime::from(approved_at),
    })
}

fn to_summary((total, count): (i64, i64)) -> DonationSummary {
    DonationSummary { total, count }
}

/// The statuses a broadcast takes donations in, as the `TEXT[]` a
/// `status = ANY($n)` condition binds.
fn open_statuses() -> Vec<&'static str> {
    let open: Vec<_> = BroadcastStatus::ALL
        .into_iter()
        .filter(|status| accepts_donations(*status))
        .collect();
    status_names(&open)
}

/// The write side of `donations` and `donation_totals`.
pub struct PgDonationRepository {
    pool: PgPool,
}

impl PgDonationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DonationRepository for PgDonationRepository {
    async fn create(&self, new: &NewDonation) -> Result<bool, PortError> {
        let inserted = sqlx::query(
            "INSERT INTO donations (id, broadcast_id, donor_id, amount) \
             SELECT $1, b.id, $3, $4 FROM broadcasts b \
             WHERE b.id = $2 AND b.deleted_at IS NULL AND b.status = ANY($5) \
               AND b.owner_id <> $3",
        )
        .bind(new.id.as_i64())
        .bind(new.broadcast_id.as_i64())
        .bind(new.donor_id.as_i64())
        .bind(new.amount.get())
        .bind(open_statuses())
        .execute(&self.pool)
        .await
        .map_err(port_error)?
        .rows_affected();
        Ok(inserted == 1)
    }

    async fn order(&self, id: DonationId) -> Result<Option<DonationOrder>, PortError> {
        type Row = (
            i64,
            i64,
            i64,
            String,
            i64,
            String,
            Option<String>,
            Option<OffsetDateTime>,
        );
        let row: Option<Row> = sqlx::query_as(
            "SELECT d.id, d.broadcast_id, d.donor_id, u.username, d.amount, d.status, \
                    d.payment_key, d.approved_at \
             FROM donations d JOIN users u ON u.id = d.donor_id WHERE d.id = $1",
        )
        .bind(id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;
        let Some((id, broadcast_id, donor_id, donor_name, amount, status, key, approved_at)) = row
        else {
            return Ok(None);
        };
        let paid = match (status.as_str(), key, approved_at) {
            (s, None, None) if s == DonationStatus::Pending.as_str() => None,
            (s, Some(key), Some(approved_at)) if s == DonationStatus::Done.as_str() => {
                Some(Payment {
                    key: PaymentKey::parse(&key).map_err(|_| invalid("payment_key", id))?,
                    approved_at: SystemTime::from(approved_at),
                })
            }
            _ => return Err(invalid("status", id)),
        };
        Ok(Some(DonationOrder {
            id: snowflake(id, "id", id)?,
            broadcast_id: snowflake(broadcast_id, "broadcast_id", id)?,
            donor_id: snowflake(donor_id, "donor_id", id)?,
            donor_name: Username::parse(&donor_name).map_err(|_| invalid("donor name", id))?,
            amount: DonationAmount::parse(amount).map_err(|_| invalid("amount", id))?,
            paid,
        }))
    }

    /// One transaction: the conditional update decides, the totals row is
    /// added to under its row lock, and the credit is announced before the
    /// commit. A concurrent settle of the same order waits on the order's
    /// row and then finds it no longer pending.
    async fn settle(
        &self,
        id: DonationId,
        payment_key: &PaymentKey,
        approved_at: SystemTime,
    ) -> Result<Settlement, PortError> {
        let mut tx = self.pool.begin().await.map_err(port_error)?;
        let settled = sqlx::query_as::<_, ViewRow>(
            "WITH d AS ( \
                 UPDATE donations SET status = 'DONE', payment_key = $2, approved_at = $3 \
                 WHERE id = $1 AND status = 'PENDING' \
                 RETURNING id, broadcast_id, donor_id, amount, approved_at \
             ) SELECT d.id, d.broadcast_id, d.amount, d.approved_at, u.username \
               FROM d JOIN users u ON u.id = d.donor_id",
        )
        .bind(id.as_i64())
        .bind(payment_key.as_str())
        .bind(OffsetDateTime::from(approved_at))
        .fetch_optional(&mut *tx)
        .await;
        let row = match settled {
            Ok(Some(row)) => row,
            Ok(None) => return Ok(Settlement::NotPending),
            Err(sqlx::Error::Database(err)) if err.code().as_deref() == Some(UNIQUE_VIOLATION) => {
                return Ok(Settlement::PaymentKeyInUse);
            }
            Err(err) => return Err(port_error(err)),
        };
        let donation = to_view(row)?;
        let summary = sqlx::query_as::<_, (i64, i64)>(
            "INSERT INTO donation_totals (broadcast_id, total, count) VALUES ($1, $2, 1) \
             ON CONFLICT (broadcast_id) DO UPDATE \
             SET total = donation_totals.total + EXCLUDED.total, \
                 count = donation_totals.count + 1 \
             RETURNING total, count",
        )
        .bind(donation.broadcast_id.as_i64())
        .bind(donation.amount.get())
        .fetch_one(&mut *tx)
        .await
        .map_err(port_error)?;
        let credit = DonationCredit {
            donation,
            summary: to_summary(summary),
        };
        let event = RealtimeEvent {
            broadcast: credit.donation.broadcast_id,
            priority: true,
            kind: EventKind::Donation(credit.clone()),
        };
        realtime::notify(&mut *tx, &event)
            .await
            .map_err(port_error)?;
        tx.commit().await.map_err(port_error)?;
        Ok(Settlement::Credited(credit))
    }
}

/// The read side: credited donations and the totals row.
pub struct PgDonationQuery {
    pool: PgPool,
}

impl PgDonationQuery {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DonationQuery for PgDonationQuery {
    async fn list(
        &self,
        broadcast: BroadcastId,
        before: Option<DonationId>,
        limit: u32,
    ) -> Result<Vec<DonationView>, PortError> {
        sqlx::query_as::<_, ViewRow>(
            "SELECT d.id, d.broadcast_id, d.amount, d.approved_at, u.username \
             FROM donations d JOIN users u ON u.id = d.donor_id \
             WHERE d.broadcast_id = $1 AND d.status = 'DONE' \
               AND ($2::BIGINT IS NULL OR d.id < $2) \
             ORDER BY d.id DESC LIMIT $3",
        )
        .bind(broadcast.as_i64())
        .bind(before.map(SnowflakeId::as_i64))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(port_error)?
        .into_iter()
        .map(to_view)
        .collect()
    }

    async fn summary(&self, broadcast: BroadcastId) -> Result<DonationSummary, PortError> {
        let row: Option<(i64, i64)> =
            sqlx::query_as("SELECT total, count FROM donation_totals WHERE broadcast_id = $1")
                .bind(broadcast.as_i64())
                .fetch_optional(&self.pool)
                .await
                .map_err(port_error)?;
        Ok(row.map(to_summary).unwrap_or_default())
    }
}
