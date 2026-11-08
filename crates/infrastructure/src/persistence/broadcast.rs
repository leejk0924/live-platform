use std::time::SystemTime;

use application::ports::{
    PortError,
    broadcast::{
        BroadcastQuery, BroadcastRepository, BroadcastView, NewBroadcast, PublisherPresence,
        TransitionRejection, WriteRejection,
    },
};
use async_trait::async_trait;
use domain::{
    auth::{UserId, Username},
    broadcast::{
        Broadcast, BroadcastDescription, BroadcastEvent, BroadcastId, BroadcastStatus,
        BroadcastTitle, Version,
    },
    id::SnowflakeId,
};
use sqlx::{PgPool, types::time::OffsetDateTime};

use super::status_names;

fn port_error(err: sqlx::Error) -> PortError {
    PortError(err.to_string())
}

/// The partial unique index that keeps one active broadcast per owner.
const ONE_ACTIVE_PER_OWNER: &str = "broadcasts_one_active_per_owner";

/// A macro rather than a const so `concat!` builds every statement at
/// compile time; no SQL is assembled at run time.
macro_rules! columns {
    () => {
        "b.id, b.owner_id, b.title, b.description, b.status, b.version, b.created_at"
    };
}

type BroadcastRow = (i64, i64, String, String, String, i32, OffsetDateTime);

fn to_broadcast(
    (id, owner_id, title, description, status, version, created_at): BroadcastRow,
) -> Result<Broadcast, PortError> {
    let invalid = |what: &str| PortError(format!("invalid stored {what} for broadcast {id}"));
    Ok(Broadcast {
        id: SnowflakeId::try_from(id).map_err(|_| invalid("id"))?,
        owner_id: SnowflakeId::try_from(owner_id).map_err(|_| invalid("owner_id"))?,
        title: BroadcastTitle::parse(&title).map_err(|_| invalid("title"))?,
        description: BroadcastDescription::parse(&description)
            .map_err(|_| invalid("description"))?,
        status: BroadcastStatus::parse(&status).ok_or_else(|| invalid("status"))?,
        version: Version::new(version).ok_or_else(|| invalid("version"))?,
        created_at: SystemTime::from(created_at),
    })
}

/// The write side of `broadcasts`.
pub struct PgBroadcastRepository {
    pool: PgPool,
}

impl PgBroadcastRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Runs only after a conditional write matched no row, to say why. The
    /// write itself already decided; this read cannot let anything through.
    async fn rejection(
        &self,
        id: BroadcastId,
        owner: UserId,
        otherwise: WriteRejection,
    ) -> Result<WriteRejection, PortError> {
        let stored_owner: Option<i64> = sqlx::query_scalar(
            "SELECT owner_id FROM broadcasts WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(match stored_owner {
            None => WriteRejection::NotFound,
            Some(stored) if stored != owner.as_i64() => WriteRejection::NotOwner,
            Some(_) => otherwise,
        })
    }

    /// Runs `sql`, a conditional status change binding `$1` id, `$2` owner,
    /// `$3` the statuses `event` may leave and `$4` the one it enters, and
    /// says why when it matched no row.
    async fn transition(
        &self,
        sql: &'static str,
        id: BroadcastId,
        owner: UserId,
        event: BroadcastEvent,
    ) -> Result<Result<Broadcast, TransitionRejection>, PortError> {
        let transition = event.transition();
        let row = sqlx::query_as::<_, BroadcastRow>(sql)
            .bind(id.as_i64())
            .bind(owner.as_i64())
            .bind(status_names(transition.from))
            .bind(transition.to.as_str())
            .fetch_optional(&self.pool)
            .await;
        let row = match row {
            Err(err)
                if err
                    .as_database_error()
                    .and_then(|db| db.constraint())
                    .is_some_and(|name| name == ONE_ACTIVE_PER_OWNER) =>
            {
                return Ok(Err(TransitionRejection::OwnerAlreadyActive));
            }
            row => row.map_err(port_error)?,
        };
        if let Some(row) = row {
            return Ok(Ok(to_broadcast(row)?));
        }
        let stored: Option<(i64, String)> = sqlx::query_as(
            "SELECT owner_id, status FROM broadcasts WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(Err(match stored {
            None => TransitionRejection::NotFound,
            Some((stored, _)) if stored != owner.as_i64() => TransitionRejection::NotOwner,
            Some((_, status)) => {
                TransitionRejection::InStatus(BroadcastStatus::parse(&status).ok_or_else(|| {
                    PortError(format!("invalid stored status for broadcast {id}"))
                })?)
            }
        }))
    }
}

#[async_trait]
impl BroadcastRepository for PgBroadcastRepository {
    async fn insert(&self, new: &NewBroadcast) -> Result<Broadcast, PortError> {
        let row: BroadcastRow = sqlx::query_as(concat!(
            "INSERT INTO broadcasts AS b (id, owner_id, title, description, status) \
             VALUES ($1, $2, $3, $4, $5) RETURNING ",
            columns!()
        ))
        .bind(new.id.as_i64())
        .bind(new.owner_id.as_i64())
        .bind(new.title.as_str())
        .bind(new.description.as_str())
        .bind(BroadcastStatus::Ready.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(port_error)?;
        to_broadcast(row)
    }

    async fn update(
        &self,
        id: BroadcastId,
        owner: UserId,
        expected: Version,
        title: &BroadcastTitle,
        description: &BroadcastDescription,
    ) -> Result<Result<Broadcast, WriteRejection>, PortError> {
        let row: Option<BroadcastRow> = sqlx::query_as(concat!(
            "UPDATE broadcasts AS b \
             SET title = $4, description = $5, version = version + 1, updated_at = now() \
             WHERE id = $1 AND owner_id = $2 AND version = $3 AND deleted_at IS NULL \
             RETURNING ",
            columns!()
        ))
        .bind(id.as_i64())
        .bind(owner.as_i64())
        .bind(expected.get())
        .bind(title.as_str())
        .bind(description.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;
        if let Some(row) = row {
            return Ok(Ok(to_broadcast(row)?));
        }
        Ok(Err(self
            .rejection(id, owner, WriteRejection::VersionMismatch)
            .await?))
    }

    async fn soft_delete(
        &self,
        id: BroadcastId,
        owner: UserId,
    ) -> Result<Result<(), WriteRejection>, PortError> {
        let result = sqlx::query(
            "UPDATE broadcasts \
             SET deleted_at = now(), version = version + 1, updated_at = now() \
             WHERE id = $1 AND owner_id = $2 AND status = ANY($3) AND deleted_at IS NULL",
        )
        .bind(id.as_i64())
        .bind(owner.as_i64())
        .bind(status_names(&BroadcastStatus::DELETABLE))
        .execute(&self.pool)
        .await
        .map_err(port_error)?;
        if result.rows_affected() == 1 {
            return Ok(Ok(()));
        }
        // Owned and present, so the status was not deletable when the write
        // ran, even if it has changed since.
        Ok(Err(self
            .rejection(id, owner, WriteRejection::NotDeletable)
            .await?))
    }

    async fn start(
        &self,
        id: BroadcastId,
        owner: UserId,
    ) -> Result<Result<Broadcast, TransitionRejection>, PortError> {
        let sql = concat!(
            "UPDATE broadcasts AS b \
             SET status = $4, started_at = now(), version = version + 1, updated_at = now() \
             WHERE id = $1 AND owner_id = $2 AND status = ANY($3) AND deleted_at IS NULL \
             RETURNING ",
            columns!()
        );
        self.transition(sql, id, owner, BroadcastEvent::Start).await
    }

    async fn stop(
        &self,
        id: BroadcastId,
        owner: UserId,
    ) -> Result<Result<Broadcast, TransitionRejection>, PortError> {
        let sql = concat!(
            "UPDATE broadcasts AS b \
             SET status = $4, ended_at = now(), interrupted_at = NULL,                  version = version + 1, updated_at = now() \
             WHERE id = $1 AND owner_id = $2 AND status = ANY($3) AND deleted_at IS NULL \
             RETURNING ",
            columns!()
        );
        self.transition(sql, id, owner, BroadcastEvent::Stop).await
    }
}

impl PgBroadcastRepository {
    /// Moves the broadcast between `LIVE` and `INTERRUPTED` as `event` says,
    /// whoever owns it. `interrupted_at` is set on entering `INTERRUPTED` and
    /// cleared on leaving it.
    async fn presence(&self, id: BroadcastId, event: BroadcastEvent) -> Result<bool, PortError> {
        let transition = event.transition();
        let result = sqlx::query(
            "UPDATE broadcasts              SET status = $3,                  interrupted_at = CASE WHEN $3 = 'INTERRUPTED' THEN now() END,                  version = version + 1, updated_at = now()              WHERE id = $1 AND status = ANY($2) AND deleted_at IS NULL",
        )
        .bind(id.as_i64())
        .bind(status_names(transition.from))
        .bind(transition.to.as_str())
        .execute(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(result.rows_affected() == 1)
    }
}

#[async_trait]
impl PublisherPresence for PgBroadcastRepository {
    async fn interrupt(&self, id: BroadcastId) -> Result<bool, PortError> {
        self.presence(id, BroadcastEvent::Interrupt).await
    }

    async fn resume(&self, id: BroadcastId) -> Result<bool, PortError> {
        self.presence(id, BroadcastEvent::Resume).await
    }
}

/// The read side of `broadcasts`, joined with the owner's name.
pub struct PgBroadcastQuery {
    pool: PgPool,
}

impl PgBroadcastQuery {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type ViewRow = (
    i64,
    i64,
    String,
    String,
    String,
    i32,
    OffsetDateTime,
    String,
);

fn to_view(
    (id, owner_id, title, description, status, version, created_at, owner_name): ViewRow,
) -> Result<BroadcastView, PortError> {
    let owner_name = Username::parse(&owner_name)
        .map_err(|_| PortError(format!("invalid stored username for user {owner_id}")))?;
    Ok(BroadcastView {
        broadcast: to_broadcast((
            id,
            owner_id,
            title,
            description,
            status,
            version,
            created_at,
        ))?,
        owner_name,
    })
}

#[async_trait]
impl BroadcastQuery for PgBroadcastQuery {
    async fn list(
        &self,
        before: Option<BroadcastId>,
        limit: u32,
    ) -> Result<Vec<BroadcastView>, PortError> {
        sqlx::query_as::<_, ViewRow>(concat!(
            "SELECT ",
            columns!(),
            ", u.username FROM broadcasts b JOIN users u ON u.id = b.owner_id \
             WHERE b.deleted_at IS NULL AND ($1::BIGINT IS NULL OR b.id < $1) \
             ORDER BY b.id DESC LIMIT $2"
        ))
        .bind(before.map(SnowflakeId::as_i64))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(port_error)?
        .into_iter()
        .map(to_view)
        .collect()
    }

    async fn get(&self, id: BroadcastId) -> Result<Option<BroadcastView>, PortError> {
        sqlx::query_as::<_, ViewRow>(concat!(
            "SELECT ",
            columns!(),
            ", u.username FROM broadcasts b JOIN users u ON u.id = b.owner_id \
             WHERE b.id = $1 AND b.deleted_at IS NULL"
        ))
        .bind(id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?
        .map(to_view)
        .transpose()
    }
}
