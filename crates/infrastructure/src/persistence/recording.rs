use std::time::{Duration, SystemTime};

use application::ports::{
    PortError,
    recording::{
        ClaimedRecording, NewRecording, RecordingFilter, RecordingQuery, RecordingRepository,
        RecordingView, SpooledSegment,
    },
};
use async_trait::async_trait;
use domain::{
    auth::Username,
    broadcast::{BroadcastId, BroadcastTitle},
    id::SnowflakeId,
    recording::{RecordingId, RecordingKey, RecordingStatus, StoreRetry},
};
use sqlx::{PgPool, types::time::OffsetDateTime};

fn port_error(err: sqlx::Error) -> PortError {
    PortError(err.to_string())
}

/// [`RecordingStatus::STORABLE`] as the `TEXT[]` a `status = ANY($n)`
/// condition binds.
fn storable() -> Vec<&'static str> {
    RecordingStatus::STORABLE
        .iter()
        .map(|status| status.as_str())
        .collect()
}

/// The key is derived from the ids rather than trusted from the row, and a
/// row whose stored key disagrees is reported instead of served.
fn stored_key(
    id: RecordingId,
    broadcast: BroadcastId,
    stored: &str,
) -> Result<RecordingKey, PortError> {
    let key = RecordingKey::new(broadcast, id);
    if key.as_str() != stored {
        return Err(PortError(format!("invalid stored key for recording {id}")));
    }
    Ok(key)
}

fn snowflake(raw: i64, what: &str) -> Result<SnowflakeId, PortError> {
    SnowflakeId::try_from(raw).map_err(|_| PortError(format!("invalid stored {what} {raw}")))
}

pub struct PgRecordingRepository {
    pool: PgPool,
}

impl PgRecordingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RecordingRepository for PgRecordingRepository {
    async fn register(&self, new: &NewRecording) -> Result<bool, PortError> {
        let duration_ms = i64::try_from(new.duration.as_millis())
            .map_err(|_| PortError(format!("duration of recording {} is too long", new.id)))?;
        let result = sqlx::query(
            "INSERT INTO recordings \
                 (id, broadcast_id, source, storage_key, status, duration_ms, recorded_at) \
             SELECT $1, b.id, $3, $4, $5, $6, now() FROM broadcasts b \
             WHERE b.id = $2 AND b.deleted_at IS NULL \
             ON CONFLICT (source) DO NOTHING",
        )
        .bind(new.id.as_i64())
        .bind(new.broadcast.as_i64())
        .bind(new.source.as_str())
        .bind(new.key.as_str())
        .bind(RecordingStatus::Pending.as_str())
        .bind(duration_ms)
        .execute(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn claim(&self, lease: Duration) -> Result<Option<ClaimedRecording>, PortError> {
        let row: Option<(i64, i64, String, String, i32)> = sqlx::query_as(
            "UPDATE recordings \
             SET attempts = attempts + 1, retry_at = now() + make_interval(secs => $2), \
                 updated_at = now() \
             WHERE status = ANY($1) AND id = ( \
                 SELECT id FROM recordings \
                 WHERE status = ANY($1) AND retry_at <= now() \
                 ORDER BY retry_at LIMIT 1 FOR UPDATE SKIP LOCKED \
             ) \
             RETURNING id, broadcast_id, source, storage_key, attempts",
        )
        .bind(storable())
        .bind(lease.as_secs_f64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;
        let Some((id, broadcast, source, key, attempts)) = row else {
            return Ok(None);
        };
        let id = snowflake(id, "recording id")?;
        let broadcast = snowflake(broadcast, "broadcast id")?;
        Ok(Some(ClaimedRecording {
            id,
            broadcast,
            source: SpooledSegment::new(source),
            key: stored_key(id, broadcast, &key)?,
            attempts: u32::try_from(attempts)
                .map_err(|_| PortError(format!("invalid stored attempts for recording {id}")))?,
        }))
    }

    async fn mark_stored(&self, id: RecordingId, size_bytes: u64) -> Result<bool, PortError> {
        let size = i64::try_from(size_bytes)
            .map_err(|_| PortError(format!("size of recording {id} is too large")))?;
        let result = sqlx::query(
            "UPDATE recordings SET status = $3, size_bytes = $2, updated_at = now() \
             WHERE id = $1 AND status = ANY($4)",
        )
        .bind(id.as_i64())
        .bind(size)
        .bind(RecordingStatus::Ready.as_str())
        .bind(storable())
        .execute(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn release_failed(
        &self,
        id: RecordingId,
        attempts: u32,
        retry: StoreRetry,
    ) -> Result<bool, PortError> {
        let query = match retry {
            StoreRetry::After(wait) => sqlx::query(
                "UPDATE recordings \
                 SET retry_at = now() + make_interval(secs => $2), updated_at = now() \
                 WHERE id = $1 AND status = ANY($3) AND attempts = $4",
            )
            .bind(id.as_i64())
            .bind(wait.as_secs_f64()),
            StoreRetry::GiveUp => sqlx::query(
                "UPDATE recordings SET status = $2, updated_at = now() \
                 WHERE id = $1 AND status = ANY($3) AND attempts = $4",
            )
            .bind(id.as_i64())
            .bind(RecordingStatus::Failed.as_str()),
        };
        let result = query
            .bind(storable())
            .bind(i64::from(attempts))
            .execute(&self.pool)
            .await
            .map_err(port_error)?;
        Ok(result.rows_affected() == 1)
    }
}

/// The read side, joined with the broadcast's title and owner. Only
/// recordings of broadcasts that are not deleted are ever read.
pub struct PgRecordingQuery {
    pool: PgPool,
}

impl PgRecordingQuery {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// A macro rather than a const so `concat!` builds every statement at
/// compile time; no SQL is assembled at run time.
macro_rules! view_from {
    () => {
        "SELECT r.id, r.broadcast_id, b.title, u.username, r.status, r.duration_ms, \
                r.size_bytes, r.recorded_at, r.storage_key \
         FROM recordings r \
         JOIN broadcasts b ON b.id = r.broadcast_id AND b.deleted_at IS NULL \
         JOIN users u ON u.id = b.owner_id "
    };
}

type ViewRow = (
    i64,
    i64,
    String,
    String,
    String,
    i64,
    Option<i64>,
    OffsetDateTime,
    String,
);

fn to_view(
    (id, broadcast, title, owner, status, duration_ms, size_bytes, recorded_at, key): ViewRow,
) -> Result<RecordingView, PortError> {
    let invalid = |what: &str| PortError(format!("invalid stored {what} for recording {id}"));
    let id = snowflake(id, "recording id")?;
    let broadcast_id = snowflake(broadcast, "broadcast id")?;
    Ok(RecordingView {
        id,
        broadcast_id,
        broadcast_title: BroadcastTitle::parse(&title).map_err(|_| invalid("title"))?,
        owner_name: Username::parse(&owner).map_err(|_| invalid("owner name"))?,
        status: RecordingStatus::parse(&status).ok_or_else(|| invalid("status"))?,
        duration: Duration::from_millis(
            u64::try_from(duration_ms).map_err(|_| invalid("duration"))?,
        ),
        size_bytes: size_bytes
            .map(u64::try_from)
            .transpose()
            .map_err(|_| invalid("size"))?,
        recorded_at: SystemTime::from(recorded_at),
        key: stored_key(id, broadcast_id, &key)?,
    })
}

#[async_trait]
impl RecordingQuery for PgRecordingQuery {
    async fn list(
        &self,
        filter: RecordingFilter,
        before: Option<RecordingId>,
        limit: u32,
    ) -> Result<Vec<RecordingView>, PortError> {
        let broadcast = match filter {
            RecordingFilter::All => None,
            RecordingFilter::Broadcast(id) => Some(id.as_i64()),
        };
        sqlx::query_as::<_, ViewRow>(concat!(
            view_from!(),
            "WHERE ($1::BIGINT IS NULL OR r.broadcast_id = $1) \
             AND ($2::BIGINT IS NULL OR r.id < $2) \
             ORDER BY r.id DESC LIMIT $3"
        ))
        .bind(broadcast)
        .bind(before.map(SnowflakeId::as_i64))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(port_error)?
        .into_iter()
        .map(to_view)
        .collect()
    }

    async fn get(&self, id: RecordingId) -> Result<Option<RecordingView>, PortError> {
        sqlx::query_as::<_, ViewRow>(concat!(view_from!(), "WHERE r.id = $1"))
            .bind(id.as_i64())
            .fetch_optional(&self.pool)
            .await
            .map_err(port_error)?
            .map(to_view)
            .transpose()
    }
}
