//! Migration 0005 against rows written before it: owners with several active
//! broadcasts keep only the newest, and the status times start out set.

use sqlx::PgPool;

/// `(status, version, ended_at set, interrupted_at = updated_at,
/// live_since = updated_at)`; an unset time compares as false.
type Stored = (String, i32, bool, bool, bool);

async fn stored(pool: &PgPool, id: i64) -> Stored {
    sqlx::query_as(
        "SELECT status, version, ended_at IS NOT NULL, \
                coalesce(interrupted_at = updated_at, false), \
                coalesce(live_since = updated_at, false) \
         FROM broadcasts WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn existing_rows_are_brought_into_the_one_active_per_owner_shape(pool: PgPool) {
    let migrator = sqlx::migrate!("./migrations");
    migrator.run_to(4, &pool).await.unwrap();
    for owner in [1, 2] {
        sqlx::query("INSERT INTO users (id, username, is_demo) VALUES ($1, $2, true)")
            .bind(owner)
            .bind(format!("owner{owner}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    for (id, owner, status) in [
        (10, 1, "INTERRUPTED"),
        (11, 1, "LIVE"),
        (20, 2, "INTERRUPTED"),
    ] {
        sqlx::query(
            "INSERT INTO broadcasts (id, owner_id, title, status, updated_at, started_at) \
             VALUES ($1, $2, 't', $3, now() - interval '10 seconds', now() - interval '1 minute')",
        )
        .bind(id)
        .bind(owner)
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
    }

    migrator.run(&pool).await.unwrap();

    assert_eq!(
        stored(&pool, 10).await,
        ("ENDED".into(), 2, true, false, false),
        "an older active broadcast of the same owner ends"
    );
    assert_eq!(
        stored(&pool, 11).await,
        ("LIVE".into(), 1, false, false, true),
        "the newest stays live"
    );
    assert_eq!(
        stored(&pool, 20).await,
        ("INTERRUPTED".into(), 1, false, true, false),
        "the only active broadcast of its owner stays"
    );
    let index: Option<String> = sqlx::query_scalar(
        "SELECT indexname::TEXT FROM pg_indexes WHERE indexname = 'broadcasts_one_active_per_owner'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(index.is_some());
}
