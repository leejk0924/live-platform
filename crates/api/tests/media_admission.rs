//! `/internal/media/auth` end to end: what MediaMTX would post, against the
//! real router, real tokens and a fresh Postgres database per test.

mod support;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use base64ct::{Base64, Encoding};
use serde_json::{Value, json};
use sqlx::PgPool;
use support::{MEDIA_SECRET, PASSWORD, Reply, publish_key, request, send, sign_up_streamer, state};

const AUTH: &str = "/internal/media/auth";

struct World {
    pool: PgPool,
    router: Router,
    owner: String,
}

impl World {
    async fn new(pool: PgPool) -> Self {
        let router = api::app(state(&pool, false).await);
        let owner = sign_up_streamer(&pool, &router, "owner", PASSWORD)
            .await
            .token();
        Self {
            pool,
            router,
            owner,
        }
    }

    async fn broadcast(&self) -> String {
        let reply = send(
            &self.router,
            request(
                Method::POST,
                "/api/broadcasts",
                Some(json!({"title": "라이브", "description": ""})),
                Some(&self.owner),
            ),
        )
        .await;
        reply.body["id"].as_str().unwrap().to_owned()
    }

    /// A publish token from `start`.
    async fn start(&self, id: &str) -> String {
        let reply = self.post(&format!("/api/broadcasts/{id}/start")).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        reply.body["token"].as_str().unwrap().to_owned()
    }

    async fn post(&self, path: &str) -> Reply {
        send(
            &self.router,
            request(Method::POST, path, None, Some(&self.owner)),
        )
        .await
    }

    async fn ask(&self, body: Value) -> StatusCode {
        ask_with(&self.router, Some(MEDIA_SECRET), body)
            .await
            .status
    }

    async fn uses(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM publish_token_uses")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

/// Posts `body` the way MediaMTX does, with `secret` as the Basic password.
async fn ask_with(router: &Router, secret: Option<&str>, body: Value) -> Reply {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(AUTH)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(secret) = secret {
        let basic = Base64::encode_string(format!("mediamtx:{secret}").as_bytes());
        builder = builder.header(header::AUTHORIZATION, format!("Basic {basic}"));
    }
    send(router, builder.body(Body::from(body.to_string())).unwrap()).await
}

fn publish(id: &str, token: &str) -> Value {
    json!({
        "user": "", "password": "", "token": token, "ip": "172.18.0.5",
        "action": "publish", "path": format!("live/{id}"), "protocol": "webrtc",
        "id": "0b9f7f4e-2a4c-4a43-9a7e-3d1c0b0b6d8e", "query": "", "userAgent": "Chrome",
    })
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_publish_token_admits_once_and_only_while_on_air(pool: PgPool) {
    let world = World::new(pool).await;
    let id = world.broadcast().await;
    let first = world.start(&id).await;
    let second = world.start(&id).await;

    assert_eq!(
        world.ask(publish(&id, &first)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        world.ask(publish(&id, &first)).await,
        StatusCode::UNAUTHORIZED,
        "the same token again is a replay"
    );

    sqlx::query(
        "UPDATE broadcasts SET status = 'INTERRUPTED', interrupted_at = now(), live_since = NULL \
         WHERE id = $1",
    )
    .bind(id.parse::<i64>().unwrap())
    .execute(&world.pool)
    .await
    .unwrap();
    let during_grace = world.start(&id).await;
    assert_eq!(
        world.ask(publish(&id, &during_grace)).await,
        StatusCode::NO_CONTENT,
        "a fresh token reconnects an interrupted broadcast"
    );

    let stopped = world.post(&format!("/api/broadcasts/{id}/stop")).await;
    assert_eq!(stopped.status, StatusCode::OK, "{}", stopped.body);
    let reply = world.post(&format!("/api/broadcasts/{id}/start")).await;
    assert_eq!(
        reply.status,
        StatusCode::CONFLICT,
        "no new token after the end"
    );
    assert_eq!(reply.code(), "BROADCAST_NOT_STARTABLE");
    let uses = world.uses().await;
    assert_eq!(
        world.ask(publish(&id, &second)).await,
        StatusCode::UNAUTHORIZED,
        "an unused token from before the stop cannot publish to the ended broadcast"
    );
    assert_eq!(world.uses().await, uses, "a refused token is not used up");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_token_publishes_only_to_its_own_broadcast(pool: PgPool) {
    let world = World::new(pool).await;
    let id = world.broadcast().await;
    let token = world.start(&id).await;
    let other = world.broadcast().await;

    for (what, body) in [
        ("other path", publish(&other, &token)),
        ("no token", publish(&id, "")),
        ("not a token", publish(&id, "abc.def.ghi")),
        (
            "reading is never asked about",
            json!({"action": "read", "path": format!("live/{id}"), "token": token}),
        ),
        (
            "a publish token is not the control token",
            json!({"action": "api", "path": "", "token": token}),
        ),
    ] {
        assert_eq!(world.ask(body).await, StatusCode::UNAUTHORIZED, "{what}");
    }
    assert_eq!(world.uses().await, 0);
    assert_eq!(
        world.ask(publish(&id, &token)).await,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_control_api_token_of_the_api_is_admitted(pool: PgPool) {
    let world = World::new(pool).await;
    let token = publish_key().control_token().unwrap();
    assert_eq!(
        world
            .ask(json!({"action": "api", "path": "", "token": token}))
            .await,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn without_the_shared_secret_nothing_is_decided(pool: PgPool) {
    let world = World::new(pool).await;
    let id = world.broadcast().await;
    let token = world.start(&id).await;

    let wrong = "f".repeat(MEDIA_SECRET.len());
    for secret in [None, Some(""), Some(wrong.as_str()), Some("0123")] {
        let reply = ask_with(&world.router, secret, publish(&id, &token)).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{secret:?}");
        assert_eq!(reply.code(), "UNAUTHENTICATED", "{secret:?}");
    }
    assert_eq!(world.uses().await, 0, "the token was never looked at");
    assert_eq!(
        world.ask(publish(&id, &token)).await,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_internal_routes_are_not_under_the_public_api(pool: PgPool) {
    let world = World::new(pool).await;
    let reply = send(
        &world.router,
        request(Method::POST, "/api/internal/media/auth", None, None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn fifty_concurrent_uses_of_one_token_admit_one(pool: PgPool) {
    let world = World::new(pool).await;
    let id = world.broadcast().await;
    let token = world.start(&id).await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..50 {
        let router = world.router.clone();
        let body = publish(&id, &token);
        tasks.spawn(async move { ask_with(&router, Some(MEDIA_SECRET), body).await.status });
    }
    let statuses = tasks.join_all().await;
    let admitted = statuses
        .iter()
        .filter(|s| **s == StatusCode::NO_CONTENT)
        .count();
    assert_eq!(admitted, 1, "{statuses:?}");
    assert!(
        statuses
            .iter()
            .all(|s| *s == StatusCode::NO_CONTENT || *s == StatusCode::UNAUTHORIZED)
    );
}
