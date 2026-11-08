//! Recordings end to end against the real router, a fresh Postgres database
//! and temporary spool and store directories: MediaMTX reports a finished
//! segment, the storer copies it, and viewers find and replay it.

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
    MEDIA_SECRET, PASSWORD, RecordingHarness, Reply, recording_state, request, send, sign_up,
};

const SEGMENT_NAME: &str = "2026-11-08_19-32-15-610734.mp4";

async fn hook(router: &Router, body: Value) -> Reply {
    let basic = Base64::encode_string(format!("mediamtx:{MEDIA_SECRET}").as_bytes());
    let request = Request::post("/internal/media/hooks")
        .header(header::AUTHORIZATION, format!("Basic {basic}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    send(router, request).await
}

async fn segment_complete(router: &Router, stream: &str, segment: &str, duration: &str) -> Reply {
    hook(
        router,
        json!({
            "event": "segment_complete",
            "path": stream,
            "segment": segment,
            "duration": duration,
        }),
    )
    .await
}

/// A signed-up owner's new broadcast id.
async fn broadcast(router: &Router, owner: &str) -> String {
    let token = sign_up(router, owner, PASSWORD).await.token();
    let reply = send(
        router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(json!({"title": "다시 볼 방송", "description": ""})),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    reply.body["id"].as_str().unwrap().to_owned()
}

/// `(id, status, duration_ms, size_bytes)` of every recording, oldest first.
async fn recordings(pool: &PgPool) -> Vec<(i64, String, i64, Option<i64>)> {
    sqlx::query_as("SELECT id, status, duration_ms, size_bytes FROM recordings ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn recorded(pool: &PgPool) -> (Router, RecordingHarness, String) {
    let (state, harness) = recording_state(pool).await;
    let router = api::app(state);
    let id = broadcast(&router, "streamer").await;
    (router, harness, id)
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_finished_segment_is_registered_once_then_stored(pool: PgPool) {
    let (router, harness, id) = recorded(&pool).await;
    let stream = format!("live/{id}");
    let segment = harness.spool_segment(&stream, SEGMENT_NAME, b"fmp4 bytes");

    for _ in 0..2 {
        let reply = segment_complete(&router, &stream, &segment, "5.9399999999999995").await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
    }
    let [(recording, status, duration_ms, None)] = &recordings(&pool).await[..] else {
        panic!("one pending recording expected");
    };
    assert_eq!((status.as_str(), *duration_ms), ("PENDING", 5_940));

    harness.store_all().await;

    assert_eq!(
        recordings(&pool).await,
        [(*recording, "READY".into(), 5_940, Some(10))]
    );
    let stored = harness
        .store
        .join(format!("recordings/{id}/{recording}.mp4"));
    assert_eq!(std::fs::read(stored).unwrap(), b"fmp4 bytes");
    assert!(
        !harness.spool.join(&stream).join(SEGMENT_NAME).exists(),
        "the spool copy is discarded"
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn segments_that_name_nothing_known_are_accepted_and_ignored(pool: PgPool) {
    let (router, harness, id) = recorded(&pool).await;
    let stream = format!("live/{id}");
    let elsewhere = harness.spool_segment("live/1", SEGMENT_NAME, b"x");
    let ignored = [
        (
            "other/1".to_owned(),
            harness.spool_segment("other/1", SEGMENT_NAME, b"x"),
        ),
        (stream.clone(), elsewhere),
        (
            stream.clone(),
            format!("{}/{stream}/../../../etc/passwd", harness.spool.display()),
        ),
        (
            "live/1".to_owned(),
            harness.spool_segment("live/1", "a.mp4", b"x"),
        ),
    ];
    for (stream, segment) in ignored {
        let reply = segment_complete(&router, &stream, &segment, "1.0").await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{stream} {segment}");
    }
    assert!(recordings(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_segment_with_an_unreadable_duration_is_refused(pool: PgPool) {
    let (router, harness, id) = recorded(&pool).await;
    let stream = format!("live/{id}");
    let segment = harness.spool_segment(&stream, SEGMENT_NAME, b"x");
    for duration in ["-1", "NaN", "inf", "", "five"] {
        let reply = segment_complete(&router, &stream, &segment, duration).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{duration:?}");
        assert_eq!(reply.code(), "INVALID_REQUEST");
    }
    let missing = hook(
        &router,
        json!({"event": "segment_complete", "path": stream, "segment": segment}),
    )
    .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert!(recordings(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_background_storer_wakes_on_a_registered_segment(pool: PgPool) {
    let (state, harness) = recording_state(&pool).await;
    let wake = std::sync::Arc::clone(&state.recordings.stored_wake);
    let router = api::app(state);
    let id = broadcast(&router, "streamer").await;
    let stream = format!("live/{id}");
    let segment = harness.spool_segment(&stream, SEGMENT_NAME, b"x");
    let storer = api::recording_store::spawn(harness.storer, wake);
    // Let the first pass find nothing, so only the wake can start the next.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    segment_complete(&router, &stream, &segment, "1.0").await;

    let ready = tokio::time::timeout(api::recording_store::STORE_TICK / 2, async {
        loop {
            if let [(_, status, _, _)] = &recordings(&pool).await[..]
                && status == "READY"
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    storer.shutdown().await;
    assert!(ready.is_ok(), "stored before the next tick");
}
