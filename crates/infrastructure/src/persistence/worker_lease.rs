use std::time::Duration;

use application::ports::id::{LeaseError, LeaseHolder, WorkerIdAllocator, WorkerLease};
use async_trait::async_trait;
use domain::id::WorkerId;
use sqlx::PgPool;

/// Worker id leases in the `worker_leases` table. Expiry is judged by the
/// database's `now()` only, because instances' clocks may disagree.
pub struct PgWorkerIdAllocator {
    pool: PgPool,
    ttl: Duration,
}

impl PgWorkerIdAllocator {
    pub fn new(pool: PgPool, ttl: Duration) -> Self {
        Self { pool, ttl }
    }
}

fn unavailable(err: sqlx::Error) -> LeaseError {
    LeaseError::Unavailable(err.to_string())
}

fn to_db_ms(ms: u64) -> Result<i64, LeaseError> {
    i64::try_from(ms).map_err(|_| LeaseError::Unavailable(format!("last_ms {ms} out of range")))
}

#[async_trait]
impl WorkerIdAllocator for PgWorkerIdAllocator {
    async fn acquire(&self) -> Result<WorkerLease, LeaseError> {
        // The outer `expires_at <= now()` re-checks the row after the lock, so
        // two callers can never both claim one slot.
        let row: Option<(i16, String, i64)> = sqlx::query_as(
            "UPDATE worker_leases w \
             SET holder = gen_random_uuid()::text, expires_at = now() + make_interval(secs => $1) \
             WHERE w.worker_id = (SELECT worker_id FROM worker_leases WHERE expires_at <= now() \
                                  ORDER BY expires_at, worker_id LIMIT 1 FOR UPDATE SKIP LOCKED) \
               AND w.expires_at <= now() \
             RETURNING worker_id, holder, last_ms",
        )
        .bind(self.ttl.as_secs_f64())
        .fetch_optional(&self.pool)
        .await
        .map_err(unavailable)?;

        let (worker_id, holder, last_ms) = row.ok_or(LeaseError::Exhausted)?;
        let worker = u16::try_from(worker_id)
            .ok()
            .and_then(|raw| WorkerId::try_from(raw).ok())
            .ok_or_else(|| LeaseError::Unavailable(format!("invalid worker_id {worker_id}")))?;
        let floor_ms = u64::try_from(last_ms)
            .map_err(|_| LeaseError::Unavailable(format!("invalid last_ms {last_ms}")))?;
        Ok(WorkerLease {
            worker,
            holder: LeaseHolder::new(holder),
            floor_ms,
        })
    }

    async fn renew(&self, lease: &WorkerLease, last_ms: u64) -> Result<(), LeaseError> {
        let result = sqlx::query(
            "UPDATE worker_leases \
             SET expires_at = now() + make_interval(secs => $4), last_ms = GREATEST(last_ms, $3) \
             WHERE worker_id = $1 AND holder = $2 AND expires_at > now()",
        )
        .bind(lease.worker.get() as i16)
        .bind(lease.holder.as_str())
        .bind(to_db_ms(last_ms)?)
        .bind(self.ttl.as_secs_f64())
        .execute(&self.pool)
        .await
        .map_err(unavailable)?;

        if result.rows_affected() == 0 {
            return Err(LeaseError::Lost);
        }
        Ok(())
    }

    async fn release(&self, lease: &WorkerLease, last_ms: u64) -> Result<(), LeaseError> {
        sqlx::query(
            "UPDATE worker_leases \
             SET holder = NULL, expires_at = now(), last_ms = $3 \
             WHERE worker_id = $1 AND holder = $2",
        )
        .bind(lease.worker.get() as i16)
        .bind(lease.holder.as_str())
        .bind(to_db_ms(last_ms)?)
        .execute(&self.pool)
        .await
        .map_err(unavailable)?;
        Ok(())
    }
}
