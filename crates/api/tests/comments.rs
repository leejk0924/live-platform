//! The comment endpoints end to end: the real router, real adapters and a
//! fresh Postgres database per test.

mod support;

use std::time::Duration;

use axum::{
    Router,
    http::{HeaderValue, Method, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use support::{PASSWORD, Reply, SseReader, request, router, send, sign_up};
use tower::ServiceExt;

struct Chat {
    router: Router,
    owner: String,
    viewer: String,
    other: String,
    broadcast: String,
}

async fn chat(pool: &PgPool) -> Chat {
    let router = router(pool, false).await;
    let owner = sign_up(&router, "owner", PASSWORD).await.token();
    let viewer = sign_up(&router, "viewer", PASSWORD).await.token();
    let other = sign_up(&router, "other", PASSWORD).await.token();
    let created = send(
        &router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(json!({"title": "방송", "description": ""})),
            Some(&owner),
        ),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let broadcast = created.body["id"].as_str().unwrap().to_owned();
    Chat {
        router,
        owner,
        viewer,
        other,
        broadcast,
    }
}

impl Chat {
    fn comments(&self) -> String {
        format!("/api/broadcasts/{}/comments", self.broadcast)
    }

    async fn post(&self, token: &str, body: &str) -> Reply {
        send(
            &self.router,
            request(
                Method::POST,
                &self.comments(),
                Some(json!({"body": body})),
                Some(token),
            ),
        )
        .await
    }

    async fn posted(&self, token: &str, body: &str) -> String {
        let reply = self.post(token, body).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        reply.body["id"].as_str().unwrap().to_owned()
    }

    async fn edit(&self, token: &str, id: &str, if_match: &str, body: &str) -> Reply {
        let mut request = request(
            Method::PUT,
            &format!("{}/{id}", self.comments()),
            Some(json!({"body": body})),
            Some(token),
        );
        request
            .headers_mut()
            .insert(header::IF_MATCH, HeaderValue::from_str(if_match).unwrap());
        send(&self.router, request).await
    }

    async fn delete(&self, token: &str, id: &str) -> Reply {
        send(
            &self.router,
            request(
                Method::DELETE,
                &format!("{}/{id}", self.comments()),
                None,
                Some(token),
            ),
        )
        .await
    }

    async fn list(&self, query: &str) -> Reply {
        send(
            &self.router,
            request(
                Method::GET,
                &format!("{}{query}", self.comments()),
                None,
                None,
            ),
        )
        .await
    }

    async fn follow(&self, last_event_id: Option<&str>) -> SseReader {
        let mut request = request(
            Method::GET,
            &format!("/api/broadcasts/{}/events", self.broadcast),
            None,
            None,
        );
        if let Some(last) = last_event_id {
            request
                .headers_mut()
                .insert("last-event-id", HeaderValue::from_str(last).unwrap());
        }
        let response = self.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        SseReader::new(response)
    }

    async fn set_slow_mode(&self, token: &str, secs: u32) -> Reply {
        send(
            &self.router,
            request(
                Method::PUT,
                &format!("/api/broadcasts/{}/chat-policy", self.broadcast),
                Some(json!({"slowModeSeconds": secs})),
                Some(token),
            ),
        )
        .await
    }
}

/// Lets the per-user interval of one second pass between posts.
async fn after_post_interval() {
    tokio::time::sleep(Duration::from_millis(1100)).await;
}

fn retry_after(reply: &Reply) -> u64 {
    reply.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn stored_body(pool: &PgPool, id: &str) -> (String, i32, bool) {
    sqlx::query_as("SELECT body, version, deleted_at IS NOT NULL FROM comments WHERE id = $1")
        .bind(id.parse::<i64>().unwrap())
        .fetch_one(pool)
        .await
        .unwrap()
}

fn bodies(reply: &Reply) -> Vec<Value> {
    reply.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["body"].clone())
        .collect()
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_comment_is_posted_edited_and_deleted_by_its_author(pool: PgPool) {
    let chat = chat(&pool).await;

    let posted = chat.post(&chat.viewer, "  첫 댓글  ").await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    assert_eq!(posted.headers[header::ETAG], "\"1\"");
    let id = posted.body["id"].as_str().unwrap().to_owned();
    assert_eq!(posted.body["body"], "첫 댓글");
    assert_eq!(posted.body["authorName"], "viewer");
    assert_eq!(posted.body["broadcastId"], chat.broadcast.as_str());
    assert_eq!(
        (&posted.body["deleted"], &posted.body["edited"]),
        (&json!(false), &json!(false))
    );
    assert!(posted.body["revision"].is_string(), "{}", posted.body);

    let edited = chat.edit(&chat.viewer, &id, "\"1\"", "고친 댓글").await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(edited.headers[header::ETAG], "\"2\"");
    assert_eq!(
        (&edited.body["body"], &edited.body["edited"]),
        (&json!("고친 댓글"), &json!(true))
    );

    let listed = chat.list("").await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(bodies(&listed), [json!("고친 댓글")]);
    assert_eq!(listed.body["revision"], edited.body["revision"]);

    let deleted = chat.delete(&chat.viewer, &id).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    assert_eq!(bodies(&chat.list("").await), Vec::<Value>::new());
    assert!(stored_body(&pool, &id).await.2, "soft deleted");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn only_the_author_may_edit_or_delete(pool: PgPool) {
    let chat = chat(&pool).await;
    let id = chat.posted(&chat.viewer, "mine").await;

    for token in [&chat.other, &chat.owner] {
        let edit = chat.edit(token, &id, "\"1\"", "hijacked").await;
        assert_eq!(edit.status, StatusCode::FORBIDDEN, "{}", edit.body);
        assert_eq!(edit.code(), "NOT_COMMENT_AUTHOR");
        let delete = chat.delete(token, &id).await;
        assert_eq!(delete.status, StatusCode::FORBIDDEN, "{}", delete.body);
        assert_eq!(delete.code(), "NOT_COMMENT_AUTHOR");
    }
    assert_eq!(
        stored_body(&pool, &id).await,
        ("mine".to_owned(), 1, false),
        "unchanged"
    );

    let anonymous = send(
        &chat.router,
        request(
            Method::POST,
            &chat.comments(),
            Some(json!({"body": "hi"})),
            None,
        ),
    )
    .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_stale_or_deleted_comment_cannot_be_edited(pool: PgPool) {
    let chat = chat(&pool).await;
    let id = chat.posted(&chat.viewer, "v1").await;
    assert_eq!(
        chat.edit(&chat.viewer, &id, "\"1\"", "v2").await.status,
        StatusCode::OK
    );

    let stale = chat.edit(&chat.viewer, &id, "\"1\"", "lost update").await;
    assert_eq!(stale.status, StatusCode::CONFLICT, "{}", stale.body);
    assert_eq!(stale.code(), "VERSION_CONFLICT");
    assert_eq!(stored_body(&pool, &id).await.0, "v2");

    let missing = chat.edit(&chat.viewer, &id, "", "x").await;
    assert_eq!(missing.code(), "INVALID_IF_MATCH");

    assert_eq!(
        chat.delete(&chat.viewer, &id).await.status,
        StatusCode::NO_CONTENT
    );
    for reply in [
        chat.edit(&chat.viewer, &id, "\"3\"", "back").await,
        chat.delete(&chat.viewer, &id).await,
        chat.edit(&chat.viewer, "123", "\"1\"", "x").await,
        chat.edit(&chat.viewer, "nope", "\"1\"", "x").await,
    ] {
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.body);
        assert_eq!(reply.code(), "COMMENT_NOT_FOUND");
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_comment_is_one_line_of_at_most_500_characters(pool: PgPool) {
    let chat = chat(&pool).await;
    for bad in ["a".repeat(501), "   ".into(), "two\nlines".into()] {
        let reply = chat.post(&chat.viewer, &bad).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad:?}");
        assert_eq!(reply.code(), "INVALID_COMMENT");
    }
    let longest = "가".repeat(500);
    let reply = chat.post(&chat.viewer, &longest).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(reply.body["body"], longest.as_str());
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_ended_or_unknown_broadcast_takes_no_comment(pool: PgPool) {
    let chat = chat(&pool).await;
    sqlx::query("UPDATE broadcasts SET status = 'ENDED', ended_at = now() WHERE id = $1")
        .bind(chat.broadcast.parse::<i64>().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let ended = chat.post(&chat.viewer, "late").await;
    assert_eq!(ended.status, StatusCode::CONFLICT, "{}", ended.body);
    assert_eq!(ended.code(), "COMMENTS_CLOSED");

    for path in ["/api/broadcasts/123/comments", "/api/broadcasts/x/comments"] {
        let reply = send(
            &chat.router,
            request(
                Method::POST,
                path,
                Some(json!({"body": "hi"})),
                Some(&chat.viewer),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(reply.code(), "BROADCAST_NOT_FOUND");
        let listed = send(&chat.router, request(Method::GET, path, None, None)).await;
        assert_eq!(listed.code(), "BROADCAST_NOT_FOUND", "{path}");
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_user_posts_at_most_once_per_second(pool: PgPool) {
    let chat = chat(&pool).await;
    chat.posted(&chat.viewer, "one").await;

    let again = chat.post(&chat.viewer, "two").await;
    assert_eq!(
        again.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        again.body
    );
    assert_eq!(again.code(), "RATE_LIMITED");
    assert_eq!(retry_after(&again), 1);
    chat.posted(&chat.other, "someone else").await;

    after_post_interval().await;
    chat.posted(&chat.viewer, "two").await;
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn slow_mode_holds_back_viewers_but_not_the_owner(pool: PgPool) {
    let chat = chat(&pool).await;
    let set = chat.set_slow_mode(&chat.owner, 60).await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.body);
    assert_eq!(set.body, json!({"slowModeSeconds": 60}));

    chat.posted(&chat.viewer, "one").await;
    chat.posted(&chat.owner, "welcome").await;
    after_post_interval().await;

    let held = chat.post(&chat.viewer, "two").await;
    assert_eq!(held.status, StatusCode::TOO_MANY_REQUESTS, "{}", held.body);
    assert!(
        (58..=60).contains(&retry_after(&held)),
        "{:?}",
        held.headers
    );
    chat.posted(&chat.owner, "the owner is not slowed down")
        .await;

    let off = chat.set_slow_mode(&chat.owner, 0).await;
    assert_eq!(off.status, StatusCode::OK);
    chat.posted(&chat.viewer, "two").await;
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn only_the_owner_sets_the_chat_policy(pool: PgPool) {
    let chat = chat(&pool).await;
    let denied = chat.set_slow_mode(&chat.viewer, 30).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.body);
    assert_eq!(denied.code(), "NOT_BROADCAST_OWNER");

    let invalid = chat.set_slow_mode(&chat.owner, 301).await;
    assert_eq!(invalid.code(), "INVALID_SLOW_MODE");

    assert_eq!(
        chat.set_slow_mode(&chat.owner, 30).await.status,
        StatusCode::OK
    );
    let policy = send(
        &chat.router,
        request(
            Method::GET,
            &format!("/api/broadcasts/{}/chat-policy", chat.broadcast),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(policy.body, json!({"slowModeSeconds": 30}));

    let broadcast = send(
        &chat.router,
        request(
            Method::GET,
            &format!("/api/broadcasts/{}", chat.broadcast),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(
        broadcast.headers[header::ETAG],
        "\"1\"",
        "slow mode is not part of the broadcast's version"
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_list_pages_newest_first(pool: PgPool) {
    let chat = chat(&pool).await;
    let mut ids = Vec::new();
    for (i, token) in [&chat.viewer, &chat.other, &chat.owner]
        .into_iter()
        .enumerate()
    {
        ids.push(chat.posted(token, &format!("c{i}")).await);
    }

    let first = chat.list("?limit=2").await;
    assert_eq!(bodies(&first), [json!("c2"), json!("c1")]);
    let next = first.body["nextBefore"].as_str().unwrap();
    assert_eq!(next, ids[1]);

    let last = chat.list(&format!("?limit=2&before={next}")).await;
    assert_eq!(bodies(&last), [json!("c0")]);
    assert_eq!(last.body["nextBefore"], Value::Null);

    for bad in ["?limit=0", "?limit=101", "?before=abc"] {
        assert_eq!(
            chat.list(bad).await.status,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn reconnecting_replays_exactly_the_changes_after_the_last_event(pool: PgPool) {
    let chat = chat(&pool).await;
    let first = chat.post(&chat.viewer, "a").await;
    let a = first.body["id"].as_str().unwrap().to_owned();
    let seen = first.body["revision"].as_str().unwrap().to_owned();
    let b = chat.posted(&chat.other, "b").await;
    assert_eq!(
        chat.edit(&chat.viewer, &a, "\"1\"", "a2").await.status,
        StatusCode::OK
    );
    assert_eq!(
        chat.delete(&chat.other, &b).await.status,
        StatusCode::NO_CONTENT
    );
    let newest = chat.list("").await.body["revision"].clone();

    let mut events = chat.follow(Some(&seen)).await;
    let replay = events.next().await.unwrap();
    assert_eq!(replay.event, "comments");
    assert_eq!(json!(replay.id), newest);
    let changes: Vec<_> = replay
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].clone(), c["body"].clone(), c["deleted"].clone()))
        .collect();
    assert_eq!(
        changes,
        [
            (json!(a), json!("a2"), json!(false)),
            (json!(b), Value::Null, json!(true)),
        ]
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_open_stream_receives_new_comments_and_policy_changes(pool: PgPool) {
    let chat = chat(&pool).await;
    let mut events = chat.follow(None).await;

    let posted = chat.post(&chat.viewer, "실시간").await;
    let live = events.next().await.unwrap();
    assert_eq!(live.event, "comments");
    assert_eq!(live.json(), json!([posted.body]));
    assert_eq!(live.id.as_deref(), posted.body["revision"].as_str());

    chat.set_slow_mode(&chat.owner, 10).await;
    let policy = events.next().await.unwrap();
    assert_eq!(
        (policy.event.as_str(), policy.json()),
        ("policy", json!({"slowModeSeconds": 10}))
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn following_an_unknown_broadcast_is_not_found(pool: PgPool) {
    let chat = chat(&pool).await;
    let reply = send(
        &chat.router,
        request(Method::GET, "/api/broadcasts/123/events", None, None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.code(), "BROADCAST_NOT_FOUND");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_comment_posted_after_listing_an_empty_chat_reaches_the_stream(pool: PgPool) {
    let chat = chat(&pool).await;
    let listed = chat.list("").await.body["revision"].clone();
    assert_eq!(listed, json!("0"), "the start of the feed");
    let posted = chat.post(&chat.viewer, "first").await;

    let mut events = chat.follow(listed.as_str()).await;
    let replay = events.next().await.expect("the comment posted in between");
    assert_eq!(replay.json(), json!([posted.body]));
}
