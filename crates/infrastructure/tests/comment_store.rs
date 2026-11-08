//! The comment adapters against Postgres: every write is one conditional
//! statement that moves the comment forward in its broadcast's change feed.

use application::ports::{
    comment::{
        ChatPolicyStore, CommentQuery, CommentRepository, CommentView, CommentWriteRejection,
        NewComment, PolicyRejection,
    },
    event_bus::{Delivery, EventBus, EventKind, EventSubscription},
};
use domain::{
    broadcast::{BroadcastStatus, Version},
    comment::{ChatPolicy, CommentBody, Revision},
    id::SnowflakeId,
};
use infrastructure::{
    persistence::{PgChatPolicyStore, PgCommentQuery, PgCommentRepository, comment_write_lock},
    realtime::PgEventBus,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{sync::Arc, time::Duration};

const OWNER: i64 = 1;
const VIEWER: i64 = 2;
const BROADCAST: i64 = 10;

fn id(raw: i64) -> SnowflakeId {
    SnowflakeId::try_from(raw).unwrap()
}

fn body(raw: &str) -> CommentBody {
    CommentBody::parse(raw).unwrap()
}

async fn seed(pool: &PgPool, status: &str) {
    for (user, name) in [(OWNER, "owner"), (VIEWER, "viewer")] {
        sqlx::query("INSERT INTO users (id, username, is_demo) VALUES ($1, $2, true)")
            .bind(user)
            .bind(name)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO broadcasts (id, owner_id, title, status, started_at, ended_at, live_since) \
         VALUES ($1, $2, 't', $3, now(), CASE WHEN $3 = 'ENDED' THEN now() END, \
                 CASE WHEN $3 = 'LIVE' THEN now() END)",
    )
    .bind(BROADCAST)
    .bind(OWNER)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

async fn post(repo: &PgCommentRepository, comment: i64, text: &str) -> Option<CommentView> {
    repo.insert(
        &NewComment {
            id: id(comment),
            broadcast_id: id(BROADCAST),
            author_id: id(VIEWER),
            body: body(text),
        },
        false,
    )
    .await
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn every_write_moves_the_comment_forward_in_the_feed(pool: PgPool) {
    seed(&pool, "READY").await;
    let repo = PgCommentRepository::new(pool.clone());
    let query = PgCommentQuery::new(pool.clone());

    let posted = post(&repo, 100, "first").await.unwrap();
    assert_eq!(posted.author_name.as_str(), "viewer");
    assert_eq!(posted.comment.version, Version::FIRST);
    assert!(!posted.comment.edited);
    post(&repo, 101, "second").await.unwrap();

    let edited = repo
        .edit(
            id(BROADCAST),
            id(100),
            id(VIEWER),
            Version::FIRST,
            &body("first!"),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(edited.comment.edited);
    assert_eq!(edited.comment.version.get(), 2);
    let deleted = repo
        .soft_delete(id(BROADCAST), id(101), id(VIEWER))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deleted.comment.body, None);

    let changes = query
        .changes_since(id(BROADCAST), posted.revision, 10)
        .await
        .unwrap();
    let seen: Vec<_> = changes
        .iter()
        .map(|v| (v.comment.id.as_i64(), v.comment.body.clone()))
        .collect();
    assert_eq!(
        seen,
        [(100, Some(body("first!"))), (101, None)],
        "the edit and the tombstone, in write order"
    );
    assert_eq!(
        query.latest_revision(id(BROADCAST)).await.unwrap(),
        deleted.revision
    );
    let listed: Vec<_> = query
        .list(id(BROADCAST), None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|v| v.comment.id.as_i64())
        .collect();
    assert_eq!(listed, [100], "deleted comments are not listed");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn rejected_writes_say_why_and_change_nothing(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let repo = PgCommentRepository::new(pool.clone());
    let query = PgCommentQuery::new(pool.clone());
    let posted = post(&repo, 100, "mine").await.unwrap();

    let b = id(BROADCAST);
    assert_eq!(
        repo.edit(b, id(100), id(OWNER), Version::FIRST, &body("x"))
            .await
            .unwrap(),
        Err(CommentWriteRejection::NotAuthor)
    );
    assert_eq!(
        repo.soft_delete(b, id(100), id(OWNER)).await.unwrap(),
        Err(CommentWriteRejection::NotAuthor)
    );
    assert_eq!(
        repo.edit(b, id(100), id(VIEWER), Version::new(2).unwrap(), &body("x"))
            .await
            .unwrap(),
        Err(CommentWriteRejection::VersionMismatch)
    );
    assert_eq!(
        repo.edit(id(11), id(100), id(VIEWER), Version::FIRST, &body("x"))
            .await
            .unwrap(),
        Err(CommentWriteRejection::NotFound),
        "under another broadcast"
    );
    assert_eq!(query.latest_revision(b).await.unwrap(), posted.revision);

    repo.soft_delete(b, id(100), id(VIEWER))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        repo.soft_delete(b, id(100), id(VIEWER)).await.unwrap(),
        Err(CommentWriteRejection::NotFound)
    );
    assert_eq!(
        repo.edit(b, id(100), id(VIEWER), Version::new(2).unwrap(), &body("x"))
            .await
            .unwrap(),
        Err(CommentWriteRejection::NotFound)
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_ended_broadcast_takes_no_comment(pool: PgPool) {
    seed(&pool, "ENDED").await;
    let repo = PgCommentRepository::new(pool.clone());
    assert_eq!(post(&repo, 100, "late").await, None);
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM comments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn slow_mode_is_set_by_the_owner_without_a_new_version(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let store = PgChatPolicyStore::new(pool.clone());
    let slow = ChatPolicy::from_secs(30).unwrap();

    assert_eq!(
        store.set(id(BROADCAST), id(VIEWER), slow).await.unwrap(),
        Err(PolicyRejection::NotOwner)
    );
    assert_eq!(
        store.set(id(99), id(OWNER), slow).await.unwrap(),
        Err(PolicyRejection::NotFound)
    );
    store
        .set(id(BROADCAST), id(OWNER), slow)
        .await
        .unwrap()
        .unwrap();

    let target = store.target(id(BROADCAST)).await.unwrap().unwrap();
    assert_eq!(
        (target.owner, target.status, target.policy),
        (id(OWNER), BroadcastStatus::Live, slow)
    );
    let version: i32 = sqlx::query_scalar("SELECT version FROM broadcasts WHERE id = $1")
        .bind(BROADCAST)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(version, 1);
}

/// Whether a session of this test's database waits for an advisory lock.
async fn someone_waits_for_a_lock(pool: &PgPool) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND NOT granted            AND database = (SELECT oid FROM pg_database WHERE datname = current_database()))",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A writer that has taken its revision but not committed must not let a
/// later writer commit a higher revision first: a follower that saw the
/// higher one would never be sent the lower.
#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_broadcasts_revisions_become_visible_in_order(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let repo = Arc::new(PgCommentRepository::new(pool.clone()));
    let query = PgCommentQuery::new(pool.clone());
    post(&repo, 100, "first").await.unwrap();

    let mut slow = pool.begin().await.unwrap();
    let (class, key) = comment_write_lock(id(BROADCAST));
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(class)
        .bind(key)
        .execute(&mut *slow)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO comments (id, broadcast_id, author_id, body) VALUES (101, $1, $2, 'slow')",
    )
    .bind(BROADCAST)
    .bind(VIEWER)
    .execute(&mut *slow)
    .await
    .unwrap();

    let fast = tokio::spawn({
        let repo = Arc::clone(&repo);
        async move { post(&repo, 102, "fast").await }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fast.is_finished() && !someone_waits_for_a_lock(&pool).await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the later writer either commits or waits");
    let listed = query.latest_revision(id(BROADCAST)).await.unwrap();
    slow.commit().await.unwrap();
    fast.await.unwrap().unwrap();

    let replayed: Vec<_> = query
        .changes_since(id(BROADCAST), listed, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|view| view.comment.id.as_i64())
        .collect();
    assert_eq!(
        replayed,
        [101, 102],
        "everything committed after the list is replayed"
    );
}

/// How many sessions of this test's database wait for a lock of any kind.
async fn lock_waiters(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND wait_event_type = 'Lock'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn until_lock_waiters(pool: &PgPool, count: i64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while lock_waiters(pool).await < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{count} sessions waiting for a lock"));
}

async fn comment_revision(subscription: &mut Box<dyn EventSubscription>) -> Revision {
    let delivery = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("a delivery within 5 s")
        .expect("the bus is open");
    match delivery {
        Delivery::Event(event) => match &event.kind {
            EventKind::Comment(view) => view.revision,
            EventKind::ChatPolicy(_) => panic!("expected a comment event"),
        },
        Delivery::Missed => panic!("expected an event, got Missed"),
    }
}

/// A write announces itself in its own transaction, so a follower on any
/// instance hears a broadcast's changes in revision order even when the
/// writer that took the lower revision is the slower one.
#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_broadcasts_comment_events_arrive_in_revision_order(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let (bus, listener) = PgEventBus::start(pool.clone()).await.unwrap();
    let mut events = bus.subscribe(id(BROADCAST));
    let repo = Arc::new(PgCommentRepository::new(pool.clone()));

    // The viewer's insert takes its revision under the write lock, then
    // waits on its foreign key to the viewer's row.
    let mut hold = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(VIEWER)
        .execute(&mut *hold)
        .await
        .unwrap();
    let slow = tokio::spawn({
        let repo = Arc::clone(&repo);
        async move { post(&repo, 100, "slow").await }
    });
    until_lock_waiters(&pool, 1).await;
    let fast = tokio::spawn({
        let repo = Arc::clone(&repo);
        async move {
            repo.insert(
                &NewComment {
                    id: id(101),
                    broadcast_id: id(BROADCAST),
                    author_id: id(OWNER),
                    body: body("fast"),
                },
                true,
            )
            .await
            .unwrap()
        }
    });
    until_lock_waiters(&pool, 2).await;
    hold.rollback().await.unwrap();

    let slow = slow.await.unwrap().unwrap().revision;
    let fast = fast.await.unwrap().unwrap().revision;
    assert!(slow < fast, "the held writer took the lower revision");
    let heard = [
        comment_revision(&mut events).await,
        comment_revision(&mut events).await,
    ];
    assert_eq!(heard, [slow, fast]);
    listener.shutdown().await;
}

/// A pool whose `pg_notify` always fails: a function of the same name in a
/// schema searched before `pg_catalog` raises instead.
async fn pool_whose_notify_fails(pool: &PgPool) -> PgPool {
    sqlx::query("CREATE SCHEMA notify_fails")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE FUNCTION notify_fails.pg_notify(TEXT, TEXT) RETURNS VOID \
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'notify refused'; END $$",
    )
    .execute(pool)
    .await
    .unwrap();
    let options = pool
        .connect_options()
        .as_ref()
        .clone()
        .options([("search_path", "notify_fails, pg_catalog, public")]);
    PgPoolOptions::new().connect_lazy_with(options)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_write_that_cannot_be_announced_is_not_stored(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let posted = post(&PgCommentRepository::new(pool.clone()), 100, "kept")
        .await
        .unwrap();
    let failing = PgCommentRepository::new(pool_whose_notify_fails(&pool).await);
    let query = PgCommentQuery::new(pool.clone());

    let inserted = failing
        .insert(
            &NewComment {
                id: id(101),
                broadcast_id: id(BROADCAST),
                author_id: id(VIEWER),
                body: body("lost"),
            },
            false,
        )
        .await;
    assert!(inserted.is_err(), "got {inserted:?}");
    let edited = failing
        .edit(
            id(BROADCAST),
            id(100),
            id(VIEWER),
            Version::FIRST,
            &body("x"),
        )
        .await;
    assert!(edited.is_err(), "got {edited:?}");
    let deleted = failing
        .soft_delete(id(BROADCAST), id(100), id(VIEWER))
        .await;
    assert!(deleted.is_err(), "got {deleted:?}");

    assert_eq!(
        query
            .changes_since(id(BROADCAST), Revision::START, 10)
            .await
            .unwrap(),
        [posted],
        "nothing but the announced comment"
    );
}
