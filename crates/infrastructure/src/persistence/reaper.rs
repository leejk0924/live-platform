use std::time::Duration;

use application::ports::{
    PortError,
    broadcast::{ReapPass, ReaperStore, Reconciled},
};
use async_trait::async_trait;
use domain::{
    broadcast::{BroadcastEvent, BroadcastId, BroadcastStatus},
    id::SnowflakeId,
};
use sqlx::{PgPool, Postgres, Transaction};

use super::{IDLE_TIMEOUT, status_names};

/// The advisory lock a pass holds; any 64-bit value no other lock uses.
pub const REAPER_LOCK_KEY: i64 = 0x6c70_5f72_6561_7072;

/// How long a used ticket is remembered after it expires, so an API host
/// whose clock runs behind the database's still refuses a replay.
const SPENT_TICKET_RETENTION: Duration = Duration::from_secs(5 * 60);

/// Where the day of the demo reset begins: midnight for the audience the
/// service runs for.
const DEMO_RESET_TIME_ZONE: &str = "Asia/Seoul";

fn port_error(err: sqlx::Error) -> PortError {
    PortError(err.to_string())
}

fn to_ids(raw: Vec<i64>) -> Result<Vec<BroadcastId>, PortError> {
    raw.into_iter()
        .map(|id| {
            SnowflakeId::try_from(id).map_err(|_| PortError(format!("invalid stored id {id}")))
        })
        .collect()
}

/// Passes run in one transaction holding `pg_try_advisory_xact_lock`: the
/// lock goes with the transaction, so a crashed instance cannot leave it held,
/// and `now()` is the pass's start for every statement in it.
pub struct PgReaperStore {
    pool: PgPool,
}

impl PgReaperStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReaperStore for PgReaperStore {
    async fn begin(&self) -> Result<Option<Box<dyn ReapPass>>, PortError> {
        let mut tx = self.pool.begin().await.map_err(port_error)?;
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
            .bind(REAPER_LOCK_KEY)
            .fetch_one(&mut *tx)
            .await
            .map_err(port_error)?;
        if !locked {
            tx.rollback().await.map_err(port_error)?;
            return Ok(None);
        }
        Ok(Some(Box::new(PgReapPass { tx })))
    }
}

struct PgReapPass {
    tx: Transaction<'static, Postgres>,
}

#[async_trait]
impl ReapPass for PgReapPass {
    async fn reconcile(
        &mut self,
        on_air: &[BroadcastId],
        grace: Duration,
    ) -> Result<Reconciled, PortError> {
        let on_air: Vec<i64> = on_air.iter().map(|id| id.as_i64()).collect();
        let resume = BroadcastEvent::Resume.transition();
        // `interrupted_at < now()`: a report that arrived after the pass
        // began is newer than what the media server was asked.
        let resumed: Vec<i64> = sqlx::query_scalar(
            "UPDATE broadcasts \
             SET status = $2, interrupted_at = NULL, live_since = now(), \
                 version = version + 1, updated_at = now() \
             WHERE status = ANY($1) AND id = ANY($3) AND interrupted_at < now() \
               AND deleted_at IS NULL \
             RETURNING id",
        )
        .bind(status_names(resume.from))
        .bind(resume.to.as_str())
        .bind(&on_air)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(port_error)?;
        let interrupt = BroadcastEvent::Interrupt.transition();
        let interrupted: Vec<i64> = sqlx::query_scalar(
            "UPDATE broadcasts \
             SET status = $2, interrupted_at = now(), live_since = NULL, \
                 version = version + 1, updated_at = now() \
             WHERE status = ANY($1) AND NOT (id = ANY($3)) \
               AND live_since < now() - make_interval(secs => $4) AND deleted_at IS NULL \
             RETURNING id",
        )
        .bind(status_names(interrupt.from))
        .bind(interrupt.to.as_str())
        .bind(&on_air)
        .bind(grace.as_secs_f64())
        .fetch_all(&mut *self.tx)
        .await
        .map_err(port_error)?;
        Ok(Reconciled {
            interrupted: to_ids(interrupted)?,
            resumed: to_ids(resumed)?,
        })
    }

    async fn not_active(&mut self, on_air: &[BroadcastId]) -> Result<Vec<BroadcastId>, PortError> {
        let on_air: Vec<i64> = on_air.iter().map(|id| id.as_i64()).collect();
        let not_active: Vec<i64> = sqlx::query_scalar(
            "SELECT on_air.id FROM unnest($1::BIGINT[]) AS on_air(id) \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM broadcasts b \
                 WHERE b.id = on_air.id AND b.status = ANY($2) AND b.deleted_at IS NULL \
             )",
        )
        .bind(&on_air)
        .bind(status_names(&BroadcastStatus::ACTIVE))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(port_error)?;
        to_ids(not_active)
    }

    async fn expire(&mut self, grace: Duration) -> Result<Vec<BroadcastId>, PortError> {
        let expire = BroadcastEvent::Expire.transition();
        let ended: Vec<i64> = sqlx::query_scalar(
            "UPDATE broadcasts \
             SET status = $2, ended_at = now(), interrupted_at = NULL, \
                 version = version + 1, updated_at = now() \
             WHERE status = ANY($1) AND interrupted_at < now() - make_interval(secs => $3) \
               AND deleted_at IS NULL \
             RETURNING id",
        )
        .bind(status_names(expire.from))
        .bind(expire.to.as_str())
        .bind(grace.as_secs_f64())
        .fetch_all(&mut *self.tx)
        .await
        .map_err(port_error)?;
        to_ids(ended)
    }

    async fn forget_spent_tickets(&mut self) -> Result<u64, PortError> {
        let deleted = sqlx::query(
            "DELETE FROM publish_token_uses WHERE expires_at < now() - make_interval(secs => $1)",
        )
        .bind(SPENT_TICKET_RETENTION.as_secs_f64())
        .execute(&mut *self.tx)
        .await
        .map_err(port_error)?;
        Ok(deleted.rows_affected())
    }

    async fn forget_expired_sessions(&mut self) -> Result<u64, PortError> {
        let deleted = sqlx::query(
            "DELETE FROM sessions \
             WHERE expires_at <= now() OR last_seen_at <= now() - make_interval(secs => $1)",
        )
        .bind(IDLE_TIMEOUT.as_secs_f64())
        .execute(&mut *self.tx)
        .await
        .map_err(port_error)?;
        Ok(deleted.rows_affected())
    }

    async fn reset_demo_broadcasts(&mut self) -> Result<Vec<BroadcastId>, PortError> {
        let reset: Vec<i64> = sqlx::query_scalar(
            "UPDATE broadcasts b \
             SET deleted_at = now(), version = b.version + 1, updated_at = now() \
             FROM users u \
             WHERE u.id = b.owner_id AND u.is_demo AND b.status = ANY($1) \
               AND b.deleted_at IS NULL \
               AND b.created_at < date_trunc('day', now() AT TIME ZONE $2) AT TIME ZONE $2 \
             RETURNING b.id",
        )
        .bind(status_names(&BroadcastStatus::DELETABLE))
        .bind(DEMO_RESET_TIME_ZONE)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(port_error)?;
        to_ids(reset)
    }

    async fn finish(self: Box<Self>) -> Result<(), PortError> {
        self.tx.commit().await.map_err(port_error)
    }
}
