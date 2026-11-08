//! Audit events end to end on the real router and a fresh Postgres database.
//! Its own test binary, and every test captures logs before its first
//! request: tracing caches per-callsite interest process-wide, so a
//! concurrent test without a subscriber could hide the request span or an
//! audit event from the scoped subscriber.

mod support;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use base64ct::{Base64, Encoding};
use serde_json::{Value, json};
use sqlx::PgPool;
use support::{
    LogBuffer, MEDIA_SECRET, PASSWORD, Reply, live_state, request, router, send, sign_up,
};

const WRONG_PASSWORD: &str = "not the password 7f3a";

/// Posts `body` to an `/internal/media` route the way MediaMTX does.
async fn from_media_server(router: &Router, path: &str, body: Value) -> Reply {
    let basic = Base64::encode_string(format!("mediamtx:{MEDIA_SECRET}").as_bytes());
    let request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Basic {basic}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    send(router, request).await
}

async fn create_broadcast(router: &Router, owner: &str) -> String {
    let created = send(
        router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(json!({"title": "라이브", "description": ""})),
            Some(owner),
        ),
    )
    .await;
    created.body["id"].as_str().unwrap().to_owned()
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn refused_logins_are_audited_without_the_password(pool: PgPool) {
    let (logs, _guard) = LogBuffer::capture();
    let router = router(&pool, false).await;
    sign_up(&router, "alice", PASSWORD).await;

    let mut codes = Vec::new();
    for _ in 0..6 {
        let body = json!({"username": "alice", "password": WRONG_PASSWORD});
        let reply = send(
            &router,
            request(Method::POST, "/api/auth/login", Some(body), None),
        )
        .await;
        codes.push(reply.code().to_owned());
    }

    assert_eq!(
        codes,
        [
            "INVALID_CREDENTIALS",
            "INVALID_CREDENTIALS",
            "INVALID_CREDENTIALS",
            "INVALID_CREDENTIALS",
            "INVALID_CREDENTIALS",
            "RATE_LIMITED",
        ]
    );
    let audits = logs.audits("login_failed");
    assert_eq!(audits.len(), 6, "{}", logs.contents());
    for (line, code) in audits.iter().zip(&codes) {
        assert!(line.contains(r#""username":"alice""#), "{line}");
        assert!(line.contains(&format!(r#""code":"{code}""#)), "{line}");
        assert!(line.contains(r#""level":"WARN""#), "{line}");
    }
    assert!(
        !logs.contents().contains(WRONG_PASSWORD),
        "the password reached the logs"
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_forbidden_answer_is_audited_with_the_signed_in_user(pool: PgPool) {
    let (logs, _guard) = LogBuffer::capture();
    let router = router(&pool, false).await;
    let owner = sign_up(&router, "owner", PASSWORD).await.token();
    let other = sign_up(&router, "other", PASSWORD).await;
    let other_id = other.body["id"].as_str().unwrap().to_owned();
    let created = send(
        &router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(json!({"title": "라이브", "description": ""})),
            Some(&owner),
        ),
    )
    .await;
    let id = created.body["id"].as_str().unwrap();

    let reply = send(
        &router,
        request(
            Method::DELETE,
            &format!("/api/broadcasts/{id}"),
            None,
            Some(&other.token()),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    let audits = logs.audits("forbidden");
    let [line] = audits.as_slice() else {
        panic!("one forbidden audit expected in {}", logs.contents());
    };
    assert!(line.contains(r#""code":"NOT_BROADCAST_OWNER""#), "{line}");
    assert!(
        line.contains(&format!(r#""user":"{other_id}""#)),
        "{other_id} missing from {line}"
    );
    let trace = reply.headers["x-trace-id"].to_str().unwrap();
    assert!(line.contains(trace), "{line}");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_owner_ending_a_broadcast_audits_the_kick(pool: PgPool) {
    let (logs, _guard) = LogBuffer::capture();
    let router = api::app(live_state(&pool, false).await.0);
    let owner = sign_up(&router, "owner", PASSWORD).await;
    let owner_id = owner.body["id"].as_str().unwrap().to_owned();
    let created = send(
        &router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(json!({"title": "라이브", "description": ""})),
            Some(&owner.token()),
        ),
    )
    .await;
    let id = created.body["id"].as_str().unwrap().to_owned();
    sqlx::query("UPDATE broadcasts SET status = 'LIVE', live_since = now() WHERE id = $1")
        .bind(id.parse::<i64>().unwrap())
        .execute(&pool)
        .await
        .unwrap();

    let reply = send(
        &router,
        request(
            Method::POST,
            &format!("/api/broadcasts/{id}/stop"),
            None,
            Some(&owner.token()),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let audits = logs.audits("publisher_kicked");
    let [line] = audits.as_slice() else {
        panic!("one publisher_kicked audit expected in {}", logs.contents());
    };
    assert!(line.contains(&format!(r#""broadcast":"{id}""#)), "{line}");
    assert!(line.contains(r#""by":"owner""#), "{line}");
    assert!(line.contains(&format!(r#""user":"{owner_id}""#)), "{line}");
    assert!(logs.audits("kick_failed").is_empty(), "{}", logs.contents());
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_username_that_cannot_be_one_stays_out_of_the_audit(pool: PgPool) {
    const MISTYPED_PASSWORD: &str = "Hunter2!Secret";
    let (logs, _guard) = LogBuffer::capture();
    let router = router(&pool, false).await;

    let body = json!({"username": MISTYPED_PASSWORD, "password": WRONG_PASSWORD});
    let reply = send(
        &router,
        request(Method::POST, "/api/auth/login", Some(body), None),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.body);
    let audits = logs.audits("login_failed");
    let [line] = audits.as_slice() else {
        panic!("one login_failed audit expected in {}", logs.contents());
    };
    assert!(line.contains(r#""username":"<invalid>""#), "{line}");
    assert!(
        !logs.contents().contains(MISTYPED_PASSWORD),
        "the mistyped password reached the logs"
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_stream_of_an_ended_broadcast_audits_the_hook_kick(pool: PgPool) {
    let (logs, _guard) = LogBuffer::capture();
    let router = api::app(live_state(&pool, false).await.0);
    let owner = sign_up(&router, "owner", PASSWORD).await.token();
    let id = create_broadcast(&router, &owner).await;
    sqlx::query("UPDATE broadcasts SET status = 'ENDED' WHERE id = $1")
        .bind(id.parse::<i64>().unwrap())
        .execute(&pool)
        .await
        .unwrap();

    let reply = from_media_server(
        &router,
        "/internal/media/hooks",
        json!({"event": "available", "path": format!("live/{id}")}),
    )
    .await;

    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    let audits = logs.audits("publisher_kicked");
    let [line] = audits.as_slice() else {
        panic!("one publisher_kicked audit expected in {}", logs.contents());
    };
    assert!(line.contains(&format!(r#""broadcast":"{id}""#)), "{line}");
    assert!(line.contains(r#""by":"hook""#), "{line}");
    assert!(logs.audits("kick_failed").is_empty(), "{}", logs.contents());
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_refused_publish_token_is_audited_without_the_token(pool: PgPool) {
    let (logs, _guard) = LogBuffer::capture();
    let router = router(&pool, false).await;
    let owner = sign_up(&router, "owner", PASSWORD).await.token();
    let id = create_broadcast(&router, &owner).await;
    let other = create_broadcast(&router, &owner).await;
    let started = send(
        &router,
        request(
            Method::POST,
            &format!("/api/broadcasts/{id}/start"),
            None,
            Some(&owner),
        ),
    )
    .await;
    let token = started.body["token"].as_str().unwrap().to_owned();

    let reply = from_media_server(
        &router,
        "/internal/media/auth",
        json!({"action": "publish", "path": format!("live/{other}"), "token": token}),
    )
    .await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.body);
    let audits = logs.audits("media_access_denied");
    let [line] = audits.as_slice() else {
        panic!(
            "one media_access_denied audit expected in {}",
            logs.contents()
        );
    };
    assert!(
        line.contains(&format!(r#""path":"live/{other}""#)),
        "{line}"
    );
    assert!(
        !logs.contents().contains(&token),
        "the publish token reached the logs"
    );
}
