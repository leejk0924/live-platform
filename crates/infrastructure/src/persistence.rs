//! PostgreSQL adapters.

mod worker_lease;

use sqlx::{PgPool, migrate::MigrateError};

pub use worker_lease::PgWorkerIdAllocator;

pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
