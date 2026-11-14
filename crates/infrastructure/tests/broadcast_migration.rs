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

/// Migration 0010: accounts that existed before roles could create
/// broadcasts, so they become streamers; the demo viewer and every account
/// created afterwards are viewers.
#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn existing_accounts_become_streamers_except_the_demo_viewer(pool: PgPool) {
    let migrator = sqlx::migrate!("./migrations");
    migrator.run_to(9, &pool).await.unwrap();
    for (id, name, demo) in [
        (1, "alice", false),
        (2, "demo-streamer", true),
        (3, "demo-viewer", true),
    ] {
        let sql = if demo {
            "INSERT INTO users (id, username, is_demo) VALUES ($1, $2, true)"
        } else {
            "INSERT INTO users (id, username, password_hash) VALUES ($1, $2, 'hash')"
        };
        sqlx::query(sql)
            .bind(id)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }

    migrator.run(&pool).await.unwrap();
    sqlx::query("INSERT INTO users (id, username, password_hash) VALUES (4, 'new', 'hash')")
        .execute(&pool)
        .await
        .unwrap();

    let roles: Vec<(String, String)> =
        sqlx::query_as("SELECT username, role FROM users ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    let roles: Vec<(&str, &str)> = roles
        .iter()
        .map(|(name, role)| (name.as_str(), role.as_str()))
        .collect();
    assert_eq!(
        roles,
        [
            ("alice", "streamer"),
            ("demo-streamer", "streamer"),
            ("demo-viewer", "viewer"),
            ("new", "viewer"),
        ]
    );
}
