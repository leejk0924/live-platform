//! The `broadcasts` table refuses exactly the titles and descriptions the
//! domain refuses, so every stored row parses when it is read back, and its
//! one-active-per-owner index covers exactly the domain's active statuses.

use domain::broadcast::{
    BroadcastDescription, BroadcastStatus, BroadcastTitle, DESCRIPTION_MAX_CHARS, TITLE_MAX_CHARS,
};
use sqlx::PgPool;

async fn insert(pool: &PgPool, id: i64, title: &str, description: &str) -> bool {
    sqlx::query(
        "INSERT INTO broadcasts (id, owner_id, title, description, status) \
         VALUES ($1, 1, $2, $3, 'READY')",
    )
    .bind(id)
    .bind(title)
    .bind(description)
    .execute(pool)
    .await
    .is_ok()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_table_accepts_exactly_what_the_domain_accepts(pool: PgPool) {
    sqlx::query("INSERT INTO users (id, username, is_demo) VALUES (1, 'owner', true)")
        .execute(&pool)
        .await
        .unwrap();

    let whitespace: String = (0..=u32::from(char::MAX))
        .filter_map(char::from_u32)
        .filter(|c| c.is_whitespace())
        .collect();
    let mut titles = vec![
        String::new(),
        "a".into(),
        "가".repeat(TITLE_MAX_CHARS),
        "가".repeat(TITLE_MAX_CHARS + 1),
        format!(" {} ", "a".repeat(TITLE_MAX_CHARS)),
        whitespace.clone(),
        format!("{whitespace}a{whitespace}"),
        "\u{200B}".into(),
    ];
    titles.extend(whitespace.chars().map(String::from));

    let mut id = 0;
    for title in &titles {
        id += 1;
        let domain = BroadcastTitle::parse(title).is_ok();
        assert_eq!(
            insert(&pool, id, title, "").await,
            domain,
            "title {title:?}"
        );
    }

    for description in [
        String::new(),
        "가".repeat(DESCRIPTION_MAX_CHARS),
        "가".repeat(DESCRIPTION_MAX_CHARS + 1),
    ] {
        id += 1;
        let domain = BroadcastDescription::parse(&description).is_ok();
        assert_eq!(
            insert(&pool, id, "ok", &description).await,
            domain,
            "description of {} chars",
            description.chars().count()
        );
    }
}

/// Two broadcasts of one owner in `status`, the second refused or not.
async fn second_one_is_refused(pool: &PgPool, status: BroadcastStatus, first_id: i64) -> bool {
    let insert = |id: i64| {
        sqlx::query(
            "INSERT INTO broadcasts (id, owner_id, title, status, interrupted_at, live_since) \
             VALUES ($1, 1, 't', $2, CASE WHEN $2 = 'INTERRUPTED' THEN now() END, \
                     CASE WHEN $2 = 'LIVE' THEN now() END)",
        )
        .bind(id)
        .bind(status.as_str())
        .execute(pool)
    };
    insert(first_id).await.unwrap();
    match insert(first_id + 1).await {
        Ok(_) => false,
        Err(err) => {
            let constraint = err.as_database_error().and_then(|db| db.constraint());
            assert_eq!(constraint, Some("broadcasts_one_active_per_owner"), "{err}");
            true
        }
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_owner_holds_at_most_one_broadcast_in_the_active_statuses(pool: PgPool) {
    sqlx::query("INSERT INTO users (id, username, is_demo) VALUES (1, 'owner', true)")
        .execute(&pool)
        .await
        .unwrap();

    let mut refused = Vec::new();
    for (i, status) in BroadcastStatus::ALL.into_iter().enumerate() {
        if second_one_is_refused(&pool, status, 10 * i as i64 + 1).await {
            refused.push(status);
        }
        sqlx::query("DELETE FROM broadcasts")
            .execute(&pool)
            .await
            .unwrap();
    }
    assert_eq!(refused, BroadcastStatus::ACTIVE);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn status_times_are_set_exactly_while_in_their_status(pool: PgPool) {
    sqlx::query("INSERT INTO users (id, username, is_demo) VALUES (1, 'owner', true)")
        .execute(&pool)
        .await
        .unwrap();
    let mut id = 0;
    for status in BroadcastStatus::ALL {
        for (interrupted_at, live_since) in [(false, false), (true, false), (false, true)] {
            id += 1;
            let stored = sqlx::query(
                "INSERT INTO broadcasts (id, owner_id, title, status, interrupted_at, live_since) \
                 VALUES ($1, 1, 't', $2, CASE WHEN $3 THEN now() END, CASE WHEN $4 THEN now() END)",
            )
            .bind(id)
            .bind(status.as_str())
            .bind(interrupted_at)
            .bind(live_since)
            .execute(&pool)
            .await
            .is_ok();
            let expected = interrupted_at == (status == BroadcastStatus::Interrupted)
                && live_since == (status == BroadcastStatus::Live);
            assert_eq!(
                stored, expected,
                "{status:?} with interrupted_at {interrupted_at}, live_since {live_since}"
            );
            sqlx::query("DELETE FROM broadcasts")
                .execute(&pool)
                .await
                .unwrap();
        }
    }
}
