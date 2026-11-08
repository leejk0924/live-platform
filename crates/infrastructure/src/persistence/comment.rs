use std::time::SystemTime;

use application::ports::{
    PortError,
    comment::{
        ChatPolicyStore, ChatTarget, CommentQuery, CommentRepository, CommentView,
        CommentWriteRejection, NewComment, PolicyRejection,
    },
    event_bus::{EventKind, RealtimeEvent},
};
use async_trait::async_trait;
use domain::{
    auth::{UserId, Username},
    broadcast::{BroadcastId, BroadcastStatus, Version},
    comment::{ChatPolicy, Comment, CommentBody, CommentId, Revision, accepts_comments},
    id::SnowflakeId,
};
use sqlx::{PgPool, types::time::OffsetDateTime};

use super::status_names;
use crate::realtime;

fn port_error(err: sqlx::Error) -> PortError {
    PortError(err.to_string())
}

/// A macro rather than a const so `concat!` builds every statement at
/// compile time; no SQL is assembled at run time. Reads from `c`, a row of
/// `comments`, joined with `u`, its author.
macro_rules! view_columns {
    () => {
        "c.id, c.broadcast_id, c.author_id, c.body, c.deleted_at IS NOT NULL, c.version, \
         c.created_at, c.updated_at IS NOT NULL, c.revision, u.username"
    };
}

type ViewRow = (
    i64,
    i64,
    i64,
    String,
    bool,
    i32,
    OffsetDateTime,
    bool,
    i64,
    String,
);

fn to_view(
    (
        id,
        broadcast_id,
        author_id,
        body,
        deleted,
        version,
        created_at,
        edited,
        revision,
        author_name,
    ): ViewRow,
) -> Result<CommentView, PortError> {
    let invalid = |what: &str| PortError(format!("invalid stored {what} for comment {id}"));
    let body = if deleted {
        None
    } else {
        Some(CommentBody::parse(&body).map_err(|_| invalid("body"))?)
    };
    Ok(CommentView {
        comment: Comment {
            id: SnowflakeId::try_from(id).map_err(|_| invalid("id"))?,
            broadcast_id: SnowflakeId::try_from(broadcast_id)
                .map_err(|_| invalid("broadcast_id"))?,
            author_id: SnowflakeId::try_from(author_id).map_err(|_| invalid("author_id"))?,
            body,
            version: Version::new(version).ok_or_else(|| invalid("version"))?,
            created_at: SystemTime::from(created_at),
            edited,
        },
        author_name: Username::parse(&author_name).map_err(|_| invalid("author name"))?,
        revision: Revision::of_change(revision).ok_or_else(|| invalid("revision"))?,
    })
}

type ViewQuery<'q> = sqlx::query::QueryAs<'q, sqlx::Postgres, ViewRow, sqlx::postgres::PgArguments>;

/// The statuses a broadcast takes comments in, as the `TEXT[]` a
/// `status = ANY($n)` condition binds.
fn open_statuses() -> Vec<&'static str> {
    let open: Vec<_> = BroadcastStatus::ALL
        .into_iter()
        .filter(|status| accepts_comments(*status))
        .collect();
    status_names(&open)
}

/// The first key of the advisory lock every write to one broadcast's
/// comments holds until it commits ("cmnt"). Two-key advisory locks never
/// collide with one-key ones such as [`super::REAPER_LOCK_KEY`].
pub const COMMENT_WRITE_LOCK_CLASS: i32 = 0x636d_6e74;

/// The advisory lock, as the `(int4, int4)` pair `pg_advisory_xact_lock`
/// takes, that serializes writes to `broadcast`'s comments. Broadcasts that
/// share the low 32 bits of their id share the lock, which only costs
/// throughput.
pub fn comment_write_lock(broadcast: BroadcastId) -> (i32, i32) {
    #[allow(clippy::cast_possible_truncation)]
    let low_bits = broadcast.as_i64() as i32;
    (COMMENT_WRITE_LOCK_CLASS, low_bits)
}

/// The write side of `comments`.
pub struct PgCommentRepository {
    pool: PgPool,
}

impl PgCommentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Runs `query`, one conditional change to `broadcast`'s comments that
    /// returns the changed row, and announces the change, holding the
    /// broadcast's write lock until it commits. The change takes its
    /// revision under the lock, so a broadcast's revisions commit, and their
    /// announcements reach listeners, in ascending order: a follower that
    /// has seen one has seen every lower one.
    async fn locked(
        &self,
        broadcast: BroadcastId,
        query: ViewQuery<'_>,
        priority: bool,
    ) -> Result<Option<CommentView>, PortError> {
        let mut tx = self.pool.begin().await.map_err(port_error)?;
        let (class, key) = comment_write_lock(broadcast);
        sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
            .bind(class)
            .bind(key)
            .execute(&mut *tx)
            .await
            .map_err(port_error)?;
        let Some(row) = query.fetch_optional(&mut *tx).await.map_err(port_error)? else {
            return Ok(None);
        };
        let view = to_view(row)?;
        let event = RealtimeEvent {
            broadcast,
            priority,
            kind: EventKind::Comment(view.clone()),
        };
        realtime::notify(&mut *tx, &event)
            .await
            .map_err(port_error)?;
        tx.commit().await.map_err(port_error)?;
        Ok(Some(view))
    }

    /// Runs `query`, a conditional change binding `$1` comment, `$2`
    /// broadcast, `$3` author and optionally more, that returns the view,
    /// and says why when it matched no row. The follow-up read cannot let
    /// anything through: the write already decided.
    async fn write(
        &self,
        query: ViewQuery<'_>,
        broadcast: BroadcastId,
        id: CommentId,
        author: UserId,
    ) -> Result<Result<CommentView, CommentWriteRejection>, PortError> {
        if let Some(view) = self.locked(broadcast, query, false).await? {
            return Ok(Ok(view));
        }
        let stored_author: Option<i64> = sqlx::query_scalar(
            "SELECT c.author_id FROM comments c JOIN broadcasts b ON b.id = c.broadcast_id \
             WHERE c.id = $1 AND c.broadcast_id = $2 \
               AND c.deleted_at IS NULL AND b.deleted_at IS NULL",
        )
        .bind(id.as_i64())
        .bind(broadcast.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(Err(match stored_author {
            None => CommentWriteRejection::NotFound,
            Some(stored) if stored != author.as_i64() => CommentWriteRejection::NotAuthor,
            Some(_) => CommentWriteRejection::VersionMismatch,
        }))
    }
}

#[async_trait]
impl CommentRepository for PgCommentRepository {
    async fn insert(
        &self,
        new: &NewComment,
        priority: bool,
    ) -> Result<Option<CommentView>, PortError> {
        let query = sqlx::query_as::<_, ViewRow>(concat!(
            "WITH c AS ( \
                 INSERT INTO comments (id, broadcast_id, author_id, body) \
                 SELECT $1, b.id, $3, $4 FROM broadcasts b \
                 WHERE b.id = $2 AND b.status = ANY($5) AND b.deleted_at IS NULL \
                 RETURNING * \
             ) SELECT ",
            view_columns!(),
            " FROM c JOIN users u ON u.id = c.author_id"
        ))
        .bind(new.id.as_i64())
        .bind(new.broadcast_id.as_i64())
        .bind(new.author_id.as_i64())
        .bind(new.body.as_str())
        .bind(open_statuses());
        self.locked(new.broadcast_id, query, priority).await
    }

    async fn edit(
        &self,
        broadcast: BroadcastId,
        id: CommentId,
        author: UserId,
        expected: Version,
        body: &CommentBody,
    ) -> Result<Result<CommentView, CommentWriteRejection>, PortError> {
        let query = sqlx::query_as::<_, ViewRow>(concat!(
            "WITH c AS ( \
                 UPDATE comments \
                 SET body = $5, version = version + 1, \
                     revision = nextval('comment_revisions'), updated_at = now() \
                 WHERE id = $1 AND broadcast_id = $2 AND author_id = $3 AND version = $4 \
                   AND deleted_at IS NULL \
                   AND EXISTS (SELECT 1 FROM broadcasts WHERE id = $2 AND deleted_at IS NULL) \
                 RETURNING * \
             ) SELECT ",
            view_columns!(),
            " FROM c JOIN users u ON u.id = c.author_id"
        ))
        .bind(id.as_i64())
        .bind(broadcast.as_i64())
        .bind(author.as_i64())
        .bind(expected.get())
        .bind(body.as_str());
        self.write(query, broadcast, id, author).await
    }

    async fn soft_delete(
        &self,
        broadcast: BroadcastId,
        id: CommentId,
        author: UserId,
    ) -> Result<Result<CommentView, CommentWriteRejection>, PortError> {
        let query = sqlx::query_as::<_, ViewRow>(concat!(
            "WITH c AS ( \
                 UPDATE comments \
                 SET deleted_at = now(), version = version + 1, \
                     revision = nextval('comment_revisions') \
                 WHERE id = $1 AND broadcast_id = $2 AND author_id = $3 \
                   AND deleted_at IS NULL \
                   AND EXISTS (SELECT 1 FROM broadcasts WHERE id = $2 AND deleted_at IS NULL) \
                 RETURNING * \
             ) SELECT ",
            view_columns!(),
            " FROM c JOIN users u ON u.id = c.author_id"
        ))
        .bind(id.as_i64())
        .bind(broadcast.as_i64())
        .bind(author.as_i64());
        // Present and by this author, yet unchanged: deleted in between.
        let rejected = self.write(query, broadcast, id, author).await?;
        Ok(rejected.map_err(|rejection| match rejection {
            CommentWriteRejection::VersionMismatch => CommentWriteRejection::NotFound,
            other => other,
        }))
    }
}

/// The read side of `comments`, joined with the author's name.
pub struct PgCommentQuery {
    pool: PgPool,
}

impl PgCommentQuery {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CommentQuery for PgCommentQuery {
    async fn list(
        &self,
        broadcast: BroadcastId,
        before: Option<CommentId>,
        limit: u32,
    ) -> Result<Vec<CommentView>, PortError> {
        sqlx::query_as::<_, ViewRow>(concat!(
            "SELECT ",
            view_columns!(),
            " FROM comments c JOIN users u ON u.id = c.author_id \
             WHERE c.broadcast_id = $1 AND c.deleted_at IS NULL \
               AND ($2::BIGINT IS NULL OR c.id < $2) \
             ORDER BY c.id DESC LIMIT $3"
        ))
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

    async fn changes_since(
        &self,
        broadcast: BroadcastId,
        after: Revision,
        limit: u32,
    ) -> Result<Vec<CommentView>, PortError> {
        sqlx::query_as::<_, ViewRow>(concat!(
            "SELECT ",
            view_columns!(),
            " FROM comments c JOIN users u ON u.id = c.author_id \
             WHERE c.broadcast_id = $1 AND c.revision > $2 \
             ORDER BY c.revision LIMIT $3"
        ))
        .bind(broadcast.as_i64())
        .bind(after.get())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(port_error)?
        .into_iter()
        .map(to_view)
        .collect()
    }

    async fn latest_revision(&self, broadcast: BroadcastId) -> Result<Revision, PortError> {
        let latest: i64 = sqlx::query_scalar(
            "SELECT coalesce(max(revision), 0) FROM comments WHERE broadcast_id = $1",
        )
        .bind(broadcast.as_i64())
        .fetch_one(&self.pool)
        .await
        .map_err(port_error)?;
        Revision::new(latest).ok_or_else(|| PortError(format!("invalid stored revision {latest}")))
    }
}

/// Slow mode and what posting checks, stored on `broadcasts`.
pub struct PgChatPolicyStore {
    pool: PgPool,
}

impl PgChatPolicyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChatPolicyStore for PgChatPolicyStore {
    async fn target(&self, broadcast: BroadcastId) -> Result<Option<ChatTarget>, PortError> {
        let row: Option<(i64, String, i32)> = sqlx::query_as(
            "SELECT owner_id, status, slow_mode_seconds FROM broadcasts \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(broadcast.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;
        let Some((owner, status, slow_mode)) = row else {
            return Ok(None);
        };
        let invalid =
            |what: &str| PortError(format!("invalid stored {what} for broadcast {broadcast}"));
        Ok(Some(ChatTarget {
            owner: SnowflakeId::try_from(owner).map_err(|_| invalid("owner_id"))?,
            status: BroadcastStatus::parse(&status).ok_or_else(|| invalid("status"))?,
            policy: u32::try_from(slow_mode)
                .ok()
                .and_then(|secs| ChatPolicy::from_secs(secs).ok())
                .ok_or_else(|| invalid("slow_mode_seconds"))?,
        }))
    }

    async fn set(
        &self,
        broadcast: BroadcastId,
        owner: UserId,
        policy: ChatPolicy,
    ) -> Result<Result<(), PolicyRejection>, PortError> {
        let secs = i32::try_from(policy.slow_mode_secs())
            .map_err(|_| PortError("slow mode out of range".into()))?;
        let result = sqlx::query(
            "UPDATE broadcasts SET slow_mode_seconds = $3 \
             WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL",
        )
        .bind(broadcast.as_i64())
        .bind(owner.as_i64())
        .bind(secs)
        .execute(&self.pool)
        .await
        .map_err(port_error)?;
        if result.rows_affected() == 1 {
            return Ok(Ok(()));
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM broadcasts WHERE id = $1 AND deleted_at IS NULL)",
        )
        .bind(broadcast.as_i64())
        .fetch_one(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(Err(if exists {
            PolicyRejection::NotOwner
        } else {
            PolicyRejection::NotFound
        }))
    }
}
