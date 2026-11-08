//! PostgreSQL adapters.

mod broadcast;
mod comment;
mod publish_ticket;
mod reaper;
mod session;
mod user;
mod worker_lease;

use std::str::FromStr;

use domain::{broadcast::BroadcastStatus, secret::Secret};
use sqlx::{
    PgPool,
    migrate::MigrateError,
    postgres::{PgConnectOptions, PgPoolOptions},
};

pub use broadcast::{PgBroadcastQuery, PgBroadcastRepository};
pub use comment::{
    COMMENT_WRITE_LOCK_CLASS, PgChatPolicyStore, PgCommentQuery, PgCommentRepository,
    comment_write_lock,
};
pub use publish_ticket::PgPublishTicketLedger;
pub use reaper::{PgReaperStore, REAPER_LOCK_KEY};
pub use session::{ABSOLUTE_TIMEOUT, IDLE_TIMEOUT, PgSessionStore};
pub use sqlx::Error as DbError;
pub use user::PgUserRepository;
pub use worker_lease::PgWorkerIdAllocator;

/// A status set as the `TEXT[]` a `status = ANY($n)` condition binds.
fn status_names(statuses: &[BroadcastStatus]) -> Vec<&'static str> {
    statuses.iter().map(|status| status.as_str()).collect()
}

pub async fn connect(
    database_url: &Secret<String>,
    max_connections: u32,
) -> Result<PgPool, DbError> {
    let options = PgConnectOptions::from_str(database_url.expose())?;
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
}

pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_url_error_does_not_echo_the_password() {
        let url = Secret::new("postgres://app:s3cr3t-pw@db:notaport/app".to_owned());
        let err = connect(&url, 1).await.unwrap_err();
        assert!(!err.to_string().contains("s3cr3t-pw"), "{err}");
    }
}
