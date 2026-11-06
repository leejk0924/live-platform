//! The broadcast endpoints end to end, centred on who may do what: the real
//! router, real adapters and a fresh Postgres database per test.

mod support;

use api::app;
use axum::{
    Router,
    http::{HeaderValue, Method, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use support::{PASSWORD, Reply, request, router, send, sign_up, state};

struct Actors {
    owner: String,
    owner_id: String,
    other: String,
}

async fn actors(router: &Router) -> Actors {
    let owner = sign_up(router, "owner", PASSWORD).await;
    let other = sign_up(router, "other", PASSWORD).await;
    Actors {
        owner: owner.token(),
        owner_id: owner.body["id"].as_str().unwrap().to_owned(),
        other: other.token(),
    }
}

fn input(title: &str) -> Value {
    json!({"title": title, "description": "설명"})
}

async fn create(router: &Router, token: &str, title: &str) -> Reply {
    send(
        router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(input(title)),
            Some(token),
        ),
    )
    .await
}

async fn created_id(router: &Router, token: &str, title: &str) -> String {
    let reply = create(router, token, title).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().to_owned()
}

async fn get(router: &Router, id: &str) -> Reply {
    send(
        router,
        request(Method::GET, &format!("/api/broadcasts/{id}"), None, None),
    )
    .await
}

async fn list(router: &Router, query: &str) -> Reply {
    list_as(router, None, query).await
}

async fn list_as(router: &Router, token: Option<&str>, query: &str) -> Reply {
    send(
        router,
        request(Method::GET, &format!("/api/broadcasts{query}"), None, token),
    )
    .await
}

async fn update(
    router: &Router,
    token: Option<&str>,
    id: &str,
    if_match: Option<&str>,
    title: &str,
) -> Reply {
    let mut request = request(
        Method::PUT,
        &format!("/api/broadcasts/{id}"),
        Some(input(title)),
        token,
    );
    if let Some(tag) = if_match {
        request
            .headers_mut()
            .insert(header::IF_MATCH, HeaderValue::from_str(tag).unwrap());
    }
    send(router, request).await
}

async fn delete(router: &Router, token: Option<&str>, id: &str) -> Reply {
    send(
        router,
        request(
            Method::DELETE,
            &format!("/api/broadcasts/{id}"),
            None,
            token,
        ),
    )
    .await
}

/// What a rejected write must leave untouched.
#[derive(Debug, PartialEq)]
struct Stored {
    title: String,
    version: i32,
    deleted: bool,
}

async fn stored(pool: &PgPool, id: &str) -> Stored {
    let (title, version, deleted) = sqlx::query_as::<_, (String, i32, bool)>(
        "SELECT title, version, deleted_at IS NOT NULL FROM broadcasts WHERE id = $1",
    )
    .bind(id.parse::<i64>().unwrap())
    .fetch_one(pool)
    .await
    .unwrap();
    Stored {
        title,
        version,
        deleted,
    }
}

async fn broadcast_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM broadcasts")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[derive(Debug, Clone, Copy)]
enum Op {
    List,
    Get,
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, Copy)]
enum Actor {
    Anonymous,
    Owner,
    Other,
}

/// For `Create`, "owner" and "other" are simply two signed-in users.
const MATRIX: [(Op, Actor, StatusCode, Option<&str>); 15] = [
    (Op::List, Actor::Anonymous, StatusCode::OK, None),
    (Op::List, Actor::Owner, StatusCode::OK, None),
    (Op::List, Actor::Other, StatusCode::OK, None),
    (Op::Get, Actor::Anonymous, StatusCode::OK, None),
    (Op::Get, Actor::Owner, StatusCode::OK, None),
    (Op::Get, Actor::Other, StatusCode::OK, None),
    (
        Op::Create,
        Actor::Anonymous,
        StatusCode::UNAUTHORIZED,
        Some("UNAUTHENTICATED"),
    ),
    (Op::Create, Actor::Owner, StatusCode::CREATED, None),
    (Op::Create, Actor::Other, StatusCode::CREATED, None),
    (
        Op::Update,
        Actor::Anonymous,
        StatusCode::UNAUTHORIZED,
        Some("UNAUTHENTICATED"),
    ),
    (Op::Update, Actor::Owner, StatusCode::OK, None),
    (
        Op::Update,
        Actor::Other,
        StatusCode::FORBIDDEN,
        Some("NOT_BROADCAST_OWNER"),
    ),
    (
        Op::Delete,
        Actor::Anonymous,
        StatusCode::UNAUTHORIZED,
        Some("UNAUTHENTICATED"),
    ),
    (Op::Delete, Actor::Owner, StatusCode::NO_CONTENT, None),
    (
        Op::Delete,
        Actor::Other,
        StatusCode::FORBIDDEN,
        Some("NOT_BROADCAST_OWNER"),
    ),
];

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn authorization_matrix(pool: PgPool) {
    let router = router(&pool, false).await;
    let actors = actors(&router).await;

    for (op, actor, expected, code) in MATRIX {
        let row = format!("{op:?} as {actor:?}");
        let token = match actor {
            Actor::Anonymous => None,
            Actor::Owner => Some(actors.owner.as_str()),
            Actor::Other => Some(actors.other.as_str()),
        };
        // A fresh broadcast per row, so an earlier row cannot decide a later one.
        let id = created_id(&router, &actors.owner, "원래 제목").await;
        let before = stored(&pool, &id).await;
        let count_before = broadcast_count(&pool).await;

        let reply = match op {
            Op::List => {
                send(
                    &router,
                    request(Method::GET, "/api/broadcasts", None, token),
                )
                .await
            }
            Op::Get => {
                send(
                    &router,
                    request(Method::GET, &format!("/api/broadcasts/{id}"), None, token),
                )
                .await
            }
            Op::Create => {
                send(
                    &router,
                    request(
                        Method::POST,
                        "/api/broadcasts",
                        Some(input("새 방송")),
                        token,
                    ),
                )
                .await
            }
            Op::Update => update(&router, token, &id, Some("\"1\""), "바뀐 제목").await,
            Op::Delete => delete(&router, token, &id).await,
        };

        assert_eq!(reply.status, expected, "{row}: {}", reply.body);
        if let Some(code) = code {
            assert_eq!(reply.code(), code, "{row}");
            assert_eq!(stored(&pool, &id).await, before, "{row} changed the row");
            assert_eq!(
                broadcast_count(&pool).await,
                count_before,
                "{row} inserted a row"
            );
        }
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn create_binds_owner_status_and_version_from_the_server_only(pool: PgPool) {
    let router = router(&pool, false).await;
    let actors = actors(&router).await;
    let forged = json!({
        "title": "  첫 방송  ",
        "description": "",
        "ownerId": "1",
        "status": "LIVE",
        "version": 9,
    });

    let reply = send(
        &router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(forged),
            Some(&actors.owner),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let id = reply.body["id"].as_str().expect("id is a string");
    assert_eq!(
        reply.headers[header::LOCATION],
        format!("/api/broadcasts/{id}")
    );
    assert_eq!(reply.headers[header::ETAG], "\"1\"");
    assert_eq!(reply.body["ownerId"], actors.owner_id.as_str());
    assert_eq!(reply.body["ownerName"], "owner");
    assert_eq!(reply.body["title"], "첫 방송");
    assert_eq!(reply.body["status"], "READY");
    assert_eq!(reply.body["version"], 1);
    let created_at = reply.body["createdAt"].as_str().unwrap();
    assert!(
        created_at.len() >= 20 && created_at.ends_with('Z'),
        "{created_at}"
    );

    let fetched = get(&router, id).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.body, reply.body);
    assert_eq!(fetched.headers[header::ETAG], "\"1\"");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn invalid_input_is_rejected_before_anything_is_written(pool: PgPool) {
    let router = router(&pool, false).await;
    let actors = actors(&router).await;
    let cases = [
        (json!({"title": "  ", "description": ""}), "INVALID_TITLE"),
        (
            json!({"title": "가".repeat(101), "description": ""}),
            "INVALID_TITLE",
        ),
        (
            json!({"title": "ok", "description": "a".repeat(2001)}),
            "INVALID_DESCRIPTION",
        ),
        (json!({"title": "ok"}), "INVALID_REQUEST"),
    ];
    for (body, code) in cases {
        let reply = send(
            &router,
            request(
                Method::POST,
                "/api/broadcasts",
                Some(body.clone()),
                Some(&actors.owner),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(reply.code(), code, "{body}");
    }
    assert_eq!(broadcast_count(&pool).await, 0);

    for id in ["123", "abc", "-1"] {
        let reply = get(&router, id).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{id}");
        assert_eq!(reply.code(), "BROADCAST_NOT_FOUND", "{id}");
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_stale_if_match_is_a_version_conflict(pool: PgPool) {
    let router = router(&pool, false).await;
    let actors = actors(&router).await;
    let id = created_id(&router, &actors.owner, "v1").await;

    let first = update(&router, Some(&actors.owner), &id, Some("\"1\""), "first").await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(first.headers[header::ETAG], "\"2\"");
    assert_eq!(first.body["version"], 2);

    let second = update(&router, Some(&actors.owner), &id, Some("\"1\""), "second").await;
    assert_eq!(second.status, StatusCode::CONFLICT, "{}", second.body);
    assert_eq!(second.code(), "VERSION_CONFLICT");
    let kept = Stored {
        title: "first".into(),
        version: 2,
        deleted: false,
    };
    assert_eq!(stored(&pool, &id).await, kept);

    for (tag, code) in [
        (None, "IF_MATCH_REQUIRED"),
        (Some("2"), "INVALID_IF_MATCH"),
        (Some("W/\"2\""), "INVALID_IF_MATCH"),
    ] {
        let reply = update(&router, Some(&actors.owner), &id, tag, "third").await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{tag:?}");
        assert_eq!(reply.code(), code, "{tag:?}");
    }
    assert_eq!(stored(&pool, &id).await, kept);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn only_ready_and_ended_broadcasts_can_be_deleted(pool: PgPool) {
    let router = router(&pool, false).await;
    let actors = actors(&router).await;

    for (status, expected) in [
        ("READY", StatusCode::NO_CONTENT),
        ("LIVE", StatusCode::CONFLICT),
        ("INTERRUPTED", StatusCode::CONFLICT),
        ("ENDED", StatusCode::NO_CONTENT),
    ] {
        let id = created_id(&router, &actors.owner, status).await;
        sqlx::query("UPDATE broadcasts SET status = $2 WHERE id = $1")
            .bind(id.parse::<i64>().unwrap())
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();

        let reply = delete(&router, Some(&actors.owner), &id).await;
        assert_eq!(reply.status, expected, "{status}: {}", reply.body);

        if expected == StatusCode::CONFLICT {
            assert_eq!(reply.code(), "BROADCAST_NOT_DELETABLE", "{status}");
            assert!(!stored(&pool, &id).await.deleted, "{status}");
            assert_eq!(get(&router, &id).await.status, StatusCode::OK, "{status}");
            continue;
        }

        assert!(stored(&pool, &id).await.deleted, "{status} is soft-deleted");
        let gone = get(&router, &id).await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND, "{status}");
        assert_eq!(gone.code(), "BROADCAST_NOT_FOUND");
        let listed = list(&router, "").await;
        assert!(
            listed.body["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["id"] != id.as_str()),
            "{status} is absent from the list"
        );
        let again = delete(&router, Some(&actors.owner), &id).await;
        assert_eq!(again.status, StatusCode::NOT_FOUND, "{status} delete twice");
        let edit = update(&router, Some(&actors.owner), &id, Some("\"2\""), "x").await;
        assert_eq!(edit.status, StatusCode::NOT_FOUND, "{status} edit deleted");
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn broadcasts_survive_a_restart(pool: PgPool) {
    let id = {
        let router = router(&pool, false).await;
        let actors = actors(&router).await;
        created_id(&router, &actors.owner, "재시작 전").await
    };
    let options = pool.connect_options();
    pool.close().await;

    let fresh = PgPoolOptions::new()
        .connect_with((*options).clone())
        .await
        .unwrap();
    let restarted = app(state(&fresh, false).await);
    let reply = get(&restarted, &id).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["title"], "재시작 전");
    assert_eq!(reply.body["ownerName"], "owner");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_list_is_public_cacheable_and_paged_newest_first(pool: PgPool) {
    let router = router(&pool, false).await;
    let actors = actors(&router).await;
    let mut ids = Vec::new();
    for title in ["one", "two", "three"] {
        ids.push(created_id(&router, &actors.owner, title).await);
    }

    let first = list(&router, "?limit=2").await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.headers[header::CACHE_CONTROL], "public, max-age=5");
    let titles = |reply: &Reply| -> Vec<String> {
        reply.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["title"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(titles(&first), ["three", "two"]);
    assert_eq!(first.body["nextBefore"], ids[1].as_str());

    let next = list(&router, &format!("?limit=2&before={}", ids[1])).await;
    assert_eq!(titles(&next), ["one"]);
    assert_eq!(next.body["nextBefore"], Value::Null);

    assert_eq!(titles(&list(&router, "").await), ["three", "two", "one"]);

    for (query, code) in [
        ("?limit=0", "INVALID_PAGE_SIZE"),
        ("?limit=51", "INVALID_PAGE_SIZE"),
        ("?limit=abc", "INVALID_QUERY"),
        ("?before=abc", "INVALID_QUERY"),
    ] {
        let reply = list(&router, query).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(reply.code(), code, "{query}");
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_signed_in_list_is_the_same_cacheable_response(pool: PgPool) {
    let router = router(&pool, false).await;
    let actors = actors(&router).await;
    created_id(&router, &actors.owner, "공개 목록").await;

    let anonymous = list(&router, "").await;
    let signed_in = list_as(&router, Some(&actors.owner), "").await;

    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.body);
    assert!(
        !signed_in.headers.contains_key(header::SET_COOKIE),
        "a shared cache would hand this cookie to everyone"
    );
    assert_eq!(
        signed_in.headers[header::CACHE_CONTROL],
        anonymous.headers[header::CACHE_CONTROL]
    );
    assert_eq!(signed_in.body, anonymous.body);
}
