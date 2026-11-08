//! The `broadcasts` table refuses exactly the titles and descriptions the
//! domain refuses, so every stored row parses when it is read back.

use domain::broadcast::{
    BroadcastDescription, BroadcastTitle, DESCRIPTION_MAX_CHARS, TITLE_MAX_CHARS,
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
