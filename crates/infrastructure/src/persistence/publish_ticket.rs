use application::ports::{
    PortError,
    media::{PublishTicket, PublishTicketLedger, RedeemRejection},
};
use async_trait::async_trait;
use domain::broadcast::{BroadcastId, BroadcastStatus};
use sqlx::{PgPool, types::time::OffsetDateTime};

use super::status_names;

fn port_error(err: sqlx::Error) -> PortError {
    PortError(err.to_string())
}

/// Publish token uses in `publish_token_uses`.
pub struct PgPublishTicketLedger {
    pool: PgPool,
}

impl PgPublishTicketLedger {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PublishTicketLedger for PgPublishTicketLedger {
    async fn redeem(
        &self,
        ticket: &PublishTicket,
        broadcast: BroadcastId,
    ) -> Result<Result<(), RedeemRejection>, PortError> {
        let inserted = sqlx::query(
            "INSERT INTO publish_token_uses (jti, broadcast_id, expires_at) \
             SELECT $1, b.id, $3 FROM broadcasts b \
             WHERE b.id = $2 AND b.owner_id = $4 AND b.status = ANY($5) AND b.deleted_at IS NULL \
             ON CONFLICT (jti) DO NOTHING",
        )
        .bind(ticket.id.as_str())
        .bind(broadcast.as_i64())
        .bind(OffsetDateTime::from(ticket.expires_at))
        .bind(ticket.publisher.as_i64())
        .bind(status_names(&BroadcastStatus::ACTIVE))
        .execute(&self.pool)
        .await
        .map_err(port_error)?;
        if inserted.rows_affected() == 1 {
            return Ok(Ok(()));
        }
        // Only says why for the logs; the insert above already refused.
        let stored: Option<(i64, String, bool)> = sqlx::query_as(
            "SELECT owner_id, status, \
                    EXISTS (SELECT 1 FROM publish_token_uses WHERE jti = $2) \
             FROM broadcasts WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(broadcast.as_i64())
        .bind(ticket.id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(Err(match stored {
            None => RedeemRejection::NotFound,
            Some((owner, _, _)) if owner != ticket.publisher.as_i64() => RedeemRejection::NotOwner,
            Some((_, _, true)) => RedeemRejection::AlreadyUsed,
            Some((_, status, false)) => {
                RedeemRejection::InStatus(BroadcastStatus::parse(&status).ok_or_else(|| {
                    PortError(format!("invalid stored status for broadcast {broadcast}"))
                })?)
            }
        }))
    }
}
