//! The comment adapters against Postgres: every write is one conditional
//! statement that moves the comment forward in its broadcast's change feed.

use application::ports::comment::{
    ChatPolicyStore, CommentQuery, CommentRepository, CommentView, CommentWriteRejection,
    NewComment, PolicyRejection,
};
use domain::{
    broadcast::{BroadcastStatus, Version},
    comment::{ChatPolicy, CommentBody},
    id::SnowflakeId,
};
use infrastructure::persistence::{PgChatPolicyStore, PgCommentQuery, PgCommentRepository};
use sqlx::PgPool;

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
    repo.insert(&NewComment {
        id: id(comment),
        broadcast_id: id(BROADCAST),
        author_id: id(VIEWER),
        body: body(text),
    })
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
        .changes_since(id(BROADCAST), Some(posted.revision), 10)
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
        Some(deleted.revision)
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
    assert_eq!(
        query.latest_revision(b).await.unwrap(),
        Some(posted.revision)
    );

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
