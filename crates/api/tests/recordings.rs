//! Recordings end to end against the real router, a fresh Postgres database
//! and temporary spool and store directories: MediaMTX reports a finished
//! segment, the storer copies it, and viewers find and replay it.

mod support;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use base64ct::{Base64, Encoding};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

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
    owned_broadcast(router, owner).await.0
}

/// A signed-up owner's new broadcast id and the owner's session token.
async fn owned_broadcast(router: &Router, owner: &str) -> (String, String) {
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
    (reply.body["id"].as_str().unwrap().to_owned(), token)
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

fn get(path: &str) -> Request<Body> {
    request(Method::GET, path, None, None)
}

/// The status, headers and raw body of a request whose body may not be
/// JSON.
async fn fetch(router: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = router.clone().oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, headers, body.to_vec())
}

/// A stored recording of a fresh broadcast: the router, the recording id,
/// the broadcast id, and its owner's token.
async fn ready_recording(pool: &PgPool, bytes: &[u8]) -> (Router, String, String, String) {
    let (state, harness) = recording_state(pool).await;
    let router = api::app(state);
    let (broadcast, token) = owned_broadcast(&router, "streamer").await;
    let stream = format!("live/{broadcast}");
    let segment = harness.spool_segment(&stream, SEGMENT_NAME, bytes);
    segment_complete(&router, &stream, &segment, "12.5").await;
    harness.store_all().await;
    let [(recording, ..)] = &recordings(pool).await[..] else {
        panic!("one recording expected");
    };
    (router, recording.to_string(), broadcast, token)
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_ready_recording_is_listed_and_replayed_with_seeking(pool: PgPool) {
    let bytes: Vec<u8> = (0..1000u32).map(|i| (i % 256) as u8).collect();
    let (router, recording, broadcast, _) = ready_recording(&pool, &bytes).await;

    let one = send(&router, get(&format!("/api/recordings/{recording}"))).await;
    assert_eq!(one.status, StatusCode::OK);
    assert_eq!(
        one.body,
        json!({
            "id": recording,
            "broadcastId": broadcast,
            "broadcastTitle": "다시 볼 방송",
            "ownerName": "streamer",
            "status": "READY",
            "durationMs": 12_500,
            "sizeBytes": 1000,
            "recordedAt": one.body["recordedAt"],
        })
    );
    assert!(one.body["recordedAt"].as_str().unwrap().ends_with('Z'));
    for list in [
        "/api/recordings".to_owned(),
        format!("/api/recordings?broadcastId={broadcast}&limit=1"),
    ] {
        let page = send(&router, get(&list)).await;
        assert_eq!(page.status, StatusCode::OK, "{list}");
        assert_eq!(page.body["items"], json!([one.body]), "{list}");
        assert_eq!(page.body["nextBefore"], Value::Null, "{list}");
    }
    let other = send(&router, get("/api/recordings?broadcastId=1")).await;
    assert_eq!(other.body["items"], json!([]));

    let replay = send(&router, get(&format!("/api/recordings/{recording}/replay"))).await;
    assert_eq!(replay.status, StatusCode::OK);
    assert_eq!(replay.headers[header::CACHE_CONTROL], "no-store");
    let url = replay.body["url"].as_str().unwrap();
    assert!(
        url.starts_with(&format!("/api/recordings/{recording}/file?expires=")),
        "{url}"
    );

    let (status, headers, body) = fetch(&router, get(url)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
    assert_eq!(headers[header::ACCEPT_RANGES], "bytes");
    assert!(body == bytes, "the stored file");

    let mut ranged = get(url);
    ranged
        .headers_mut()
        .insert(header::RANGE, "bytes=100-199".parse().unwrap());
    let (status, headers, body) = fetch(&router, ranged).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers[header::CONTENT_RANGE], "bytes 100-199/1000");
    assert!(body == bytes[100..200], "the requested range");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_file_url_must_carry_its_own_valid_signature(pool: PgPool) {
    let (router, recording, ..) = ready_recording(&pool, b"x").await;
    let replay = send(&router, get(&format!("/api/recordings/{recording}/replay"))).await;
    let url = replay.body["url"].as_str().unwrap().to_owned();
    let (path, query) = url.split_once('?').unwrap();
    let expires: u64 = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("expires="))
        .unwrap()
        .parse()
        .unwrap();

    for forged in [
        format!(
            "{path}?{}",
            query.replace(
                &format!("expires={expires}"),
                &format!("expires={}", expires + 1)
            )
        ),
        format!("{path}?expires={expires}&signature=AAAA"),
        format!("{path}?expires={}&signature=AAAA", u64::MAX),
        format!("{path}?expires={expires}"),
        path.to_owned(),
        format!("/api/recordings/1/file?{query}"),
    ] {
        let reply = send(&router, get(&forged)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{forged}");
        assert_eq!(reply.code(), "INVALID_SIGNATURE", "{forged}");
    }
    let past = format!("{path}?expires=1&signature=AAAA");
    assert_eq!(
        send(&router, get(&past)).await.status,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn deleting_the_broadcast_cuts_off_its_recordings_at_once(pool: PgPool) {
    let (router, recording, broadcast, token) = ready_recording(&pool, b"x").await;
    let replay = send(&router, get(&format!("/api/recordings/{recording}/replay"))).await;
    let url = replay.body["url"].as_str().unwrap().to_owned();
    assert_eq!(fetch(&router, get(&url)).await.0, StatusCode::OK);

    let deleted = send(
        &router,
        request(
            Method::DELETE,
            &format!("/api/broadcasts/{broadcast}"),
            None,
            Some(&token),
        ),
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    for path in [
        format!("/api/recordings/{recording}"),
        format!("/api/recordings/{recording}/replay"),
        url,
    ] {
        let reply = send(&router, get(&path)).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(reply.code(), "RECORDING_NOT_FOUND", "{path}");
    }
    let page = send(&router, get("/api/recordings")).await;
    assert_eq!(page.body["items"], json!([]));
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_recording_not_yet_stored_cannot_be_replayed(pool: PgPool) {
    let (router, harness, id) = recorded(&pool).await;
    let stream = format!("live/{id}");
    let segment = harness.spool_segment(&stream, SEGMENT_NAME, b"x");
    segment_complete(&router, &stream, &segment, "1.0").await;
    let [(recording, ..)] = &recordings(&pool).await[..] else {
        panic!("one recording expected");
    };

    let one = send(&router, get(&format!("/api/recordings/{recording}"))).await;
    assert_eq!(one.body["status"], "PENDING");
    assert_eq!(one.body["sizeBytes"], Value::Null);
    let replay = send(&router, get(&format!("/api/recordings/{recording}/replay"))).await;
    assert_eq!(replay.status, StatusCode::CONFLICT);
    assert_eq!(replay.code(), "RECORDING_NOT_READY");
    for missing in ["/api/recordings/999/replay", "/api/recordings/abc"] {
        let reply = send(&router, get(missing)).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{missing}");
        assert_eq!(reply.code(), "RECORDING_NOT_FOUND", "{missing}");
    }
    for bad in ["/api/recordings?limit=0", "/api/recordings?broadcastId=x"] {
        assert_eq!(
            send(&router, get(bad)).await.status,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
}
