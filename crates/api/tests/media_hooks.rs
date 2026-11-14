//! `/internal/media/hooks` end to end: MediaMTX's reports that a publisher
//! arrived or left, against the real router and a fresh Postgres database.

mod support;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use base64ct::{Base64, Encoding};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::atomic::Ordering;

use support::{MEDIA_SECRET, PASSWORD, Reply, live_state, request, send, sign_up_streamer, state};

async fn hook_with(router: &Router, secret: Option<&str>, body: Value) -> Reply {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/internal/media/hooks");
    if let Some(secret) = secret {
        let basic = Base64::encode_string(format!("mediamtx:{secret}").as_bytes());
        builder = builder.header(header::AUTHORIZATION, format!("Basic {basic}"));
    }
    send(router, builder.body(Body::from(body.to_string())).unwrap()).await
}

async fn hook(router: &Router, event: &str, id: &str) -> StatusCode {
    hook_with(
        router,
        Some(MEDIA_SECRET),
        json!({"event": event, "path": format!("live/{id}")}),
    )
    .await
    .status
}

/// `(status, version, interrupted_at is set)`.
async fn stored(pool: &PgPool, id: &str) -> (String, i32, bool) {
    sqlx::query_as(
        "SELECT status, version, interrupted_at IS NOT NULL FROM broadcasts WHERE id = $1",
    )
    .bind(id.parse::<i64>().unwrap())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn broadcast_in(router: &Router, pool: &PgPool, owner: &str, status: &str) -> String {
    let reply = send(
        router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(json!({"title": "t", "description": ""})),
            Some(owner),
        ),
    )
    .await;
    let id = reply.body["id"].as_str().unwrap().to_owned();
    sqlx::query(
        "UPDATE broadcasts SET status = 'ENDED', interrupted_at = NULL, live_since = NULL \
         WHERE status IN ('LIVE', 'INTERRUPTED')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE broadcasts SET status = $2, \
         interrupted_at = CASE WHEN $2 = 'INTERRUPTED' THEN now() END, \
         live_since = CASE WHEN $2 = 'LIVE' THEN now() END \
         WHERE id = $1",
    )
    .bind(id.parse::<i64>().unwrap())
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
    id
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_publisher_leaving_interrupts_and_coming_back_resumes(pool: PgPool) {
    let router = api::app(state(&pool, false).await);
    let owner = sign_up_streamer(&pool, &router, "owner", PASSWORD)
        .await
        .token();
    let id = broadcast_in(&router, &pool, &owner, "LIVE").await;

    assert_eq!(
        hook(&router, "unavailable", &id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored(&pool, &id).await, ("INTERRUPTED".into(), 2, true));
    assert_eq!(
        hook(&router, "unavailable", &id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        stored(&pool, &id).await,
        ("INTERRUPTED".into(), 2, true),
        "a duplicate report changes nothing"
    );

    assert_eq!(
        hook(&router, "available", &id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored(&pool, &id).await, ("LIVE".into(), 3, false));
    assert_eq!(
        hook(&router, "available", &id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored(&pool, &id).await, ("LIVE".into(), 3, false));
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn reports_never_move_a_ready_or_ended_broadcast_and_its_stream_is_kicked(pool: PgPool) {
    let (state, kicks) = live_state(&pool, false).await;
    let router = api::app(state);
    let owner = sign_up_streamer(&pool, &router, "owner", PASSWORD)
        .await
        .token();
    for status in ["READY", "ENDED"] {
        let id = broadcast_in(&router, &pool, &owner, status).await;
        let before = stored(&pool, &id).await;
        for (event, kicked) in [("available", 1), ("unavailable", 0)] {
            let kicks_before = kicks.kicks.load(Ordering::SeqCst);
            assert_eq!(
                hook(&router, event, &id).await,
                StatusCode::NO_CONTENT,
                "{status} {event}"
            );
            assert_eq!(stored(&pool, &id).await, before, "{status} {event}");
            assert_eq!(
                kicks.kicks.load(Ordering::SeqCst) - kicks_before,
                kicked,
                "{status} {event}"
            );
        }
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn hooks_need_the_secret_and_a_known_shape(pool: PgPool) {
    let router = api::app(state(&pool, false).await);
    let owner = sign_up_streamer(&pool, &router, "owner", PASSWORD)
        .await
        .token();
    let id = broadcast_in(&router, &pool, &owner, "LIVE").await;
    let body = json!({"event": "unavailable", "path": format!("live/{id}")});

    for secret in [None, Some("wrong")] {
        let reply = hook_with(&router, secret, body.clone()).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{secret:?}");
    }
    for bad in [
        json!({"event": "ready", "path": format!("live/{id}")}),
        json!({"path": format!("live/{id}")}),
        json!("live"),
    ] {
        let reply = hook_with(&router, Some(MEDIA_SECRET), bad.clone()).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(
        stored(&pool, &id).await,
        ("LIVE".into(), 1, false),
        "nothing changed"
    );
    for path in ["other/1", "live/abc", "live/007"] {
        let reply = hook_with(
            &router,
            Some(MEDIA_SECRET),
            json!({"event": "unavailable", "path": path}),
        )
        .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{path}");
    }
}
