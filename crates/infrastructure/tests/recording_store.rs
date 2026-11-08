//! The recording adapters against Postgres: registering is idempotent per
//! segment, claims never hand one recording to two storers, every status
//! change applies only to a pending recording under its latest claim, and a
//! deleted broadcast's recordings cannot be read.

use std::time::Duration;

use application::ports::recording::{
    NewRecording, RecordingFilter, RecordingQuery, RecordingRepository, SpooledSegment,
};
use domain::{
    id::SnowflakeId,
    recording::{MAX_STORE_ATTEMPTS, RecordingKey, RecordingStatus, StoreRetry},
};
use infrastructure::persistence::{PgRecordingQuery, PgRecordingRepository};
use sqlx::PgPool;

const OWNER: i64 = 1;
const BROADCAST: i64 = 10;
const OTHER_BROADCAST: i64 = 11;
const LEASE: Duration = Duration::from_secs(600);
/// The attempt count of a recording no storer has claimed yet.
const UNCLAIMED: u32 = 0;

fn id(raw: i64) -> SnowflakeId {
    SnowflakeId::try_from(raw).unwrap()
}

async fn seed(pool: &PgPool) {
    sqlx::query("INSERT INTO users (id, username, is_demo) VALUES ($1, 'owner', true)")
        .bind(OWNER)
        .execute(pool)
        .await
        .unwrap();
    for (broadcast, title) in [(BROADCAST, "first"), (OTHER_BROADCAST, "second")] {
        sqlx::query(
            "INSERT INTO broadcasts (id, owner_id, title, status, started_at, ended_at) \
             VALUES ($1, $2, $3, 'ENDED', now(), now())",
        )
        .bind(broadcast)
        .bind(OWNER)
        .bind(title)
        .execute(pool)
        .await
        .unwrap();
    }
}

fn segment(recording: i64, broadcast: i64) -> NewRecording {
    NewRecording {
        id: id(recording),
        broadcast: id(broadcast),
        source: SpooledSegment::new(format!("live/{broadcast}/seg-{recording}.mp4")),
        key: RecordingKey::new(id(broadcast), id(recording)),
        duration: Duration::from_millis(5_940),
    }
}

async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM recordings")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn status_of(pool: &PgPool, recording: i64) -> (String, Option<i64>, i32) {
    sqlx::query_as("SELECT status, size_bytes, attempts FROM recordings WHERE id = $1")
        .bind(recording)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn delete_broadcast(pool: &PgPool, broadcast: i64) {
    sqlx::query("UPDATE broadcasts SET deleted_at = now() WHERE id = $1")
        .bind(broadcast)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_segment_reported_twice_is_one_recording(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());

    assert!(repo.register(&segment(100, BROADCAST)).await.unwrap());
    let again = NewRecording {
        id: id(101),
        key: RecordingKey::new(id(BROADCAST), id(101)),
        ..segment(100, BROADCAST)
    };
    assert!(!repo.register(&again).await.unwrap(), "same source");

    assert_eq!(count(&pool).await, 1);
    let stored: (String, i64, String) =
        sqlx::query_as("SELECT status, duration_ms, storage_key FROM recordings WHERE id = 100")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stored,
        ("PENDING".into(), 5_940, "recordings/10/100.mp4".into())
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_segment_of_an_unknown_or_deleted_broadcast_is_not_registered(pool: PgPool) {
    seed(&pool).await;
    delete_broadcast(&pool, OTHER_BROADCAST).await;
    let repo = PgRecordingRepository::new(pool.clone());

    assert!(!repo.register(&segment(100, 999)).await.unwrap(), "unknown");
    assert!(
        !repo.register(&segment(101, OTHER_BROADCAST)).await.unwrap(),
        "deleted"
    );
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn each_claim_takes_a_different_due_recording(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());
    for recording in [100, 101, 102] {
        repo.register(&segment(recording, BROADCAST)).await.unwrap();
    }
    sqlx::query("UPDATE recordings SET retry_at = now() + interval '1 minute' WHERE id = 102")
        .execute(&pool)
        .await
        .unwrap();

    let first = repo.claim(LEASE).await.unwrap().unwrap();
    let second = repo.claim(LEASE).await.unwrap().unwrap();
    assert_ne!(first.id, second.id);
    assert!(
        repo.claim(LEASE).await.unwrap().is_none(),
        "102 is not due and the others are leased"
    );
    assert_eq!(first.attempts, 1);
    assert_eq!(
        first.key,
        RecordingKey::new(id(BROADCAST), first.id),
        "{first:?}"
    );
    assert_eq!(
        first.source.as_str(),
        format!("live/{BROADCAST}/seg-{}.mp4", first.id)
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_claim_skips_a_recording_another_storer_is_claiming(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());
    repo.register(&segment(100, BROADCAST)).await.unwrap();

    let mut other = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM recordings WHERE id = 100 FOR UPDATE")
        .execute(&mut *other)
        .await
        .unwrap();
    let claimed = tokio::time::timeout(Duration::from_secs(5), repo.claim(LEASE))
        .await
        .expect("a locked row is skipped, not waited for")
        .unwrap();
    assert!(claimed.is_none());

    other.rollback().await.unwrap();
    assert_eq!(repo.claim(LEASE).await.unwrap().unwrap().id, id(100));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn only_a_pending_recording_is_marked_stored(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());
    repo.register(&segment(100, BROADCAST)).await.unwrap();
    repo.register(&segment(101, BROADCAST)).await.unwrap();
    assert!(
        repo.release_failed(id(101), UNCLAIMED, StoreRetry::GiveUp)
            .await
            .unwrap()
    );

    assert!(repo.mark_stored(id(100), 1_234).await.unwrap());
    assert!(
        !repo.mark_stored(id(100), 9).await.unwrap(),
        "already ready"
    );
    assert!(!repo.mark_stored(id(101), 9).await.unwrap(), "failed");
    assert!(
        !repo
            .release_failed(id(100), UNCLAIMED, StoreRetry::GiveUp)
            .await
            .unwrap(),
        "a ready recording stays ready"
    );

    assert_eq!(
        status_of(&pool, 100).await,
        ("READY".into(), Some(1_234), 0)
    );
    assert_eq!(status_of(&pool, 101).await, ("FAILED".into(), None, 0));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_stale_storer_cannot_touch_a_newer_claim(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());
    repo.register(&segment(100, BROADCAST)).await.unwrap();

    let stale = repo.claim(Duration::ZERO).await.unwrap().unwrap();
    let current = repo.claim(LEASE).await.unwrap().expect("the lease ran out");
    assert!(
        !repo
            .release_failed(stale.id, stale.attempts, StoreRetry::GiveUp)
            .await
            .unwrap(),
        "the stale storer's failure"
    );
    assert!(repo.mark_stored(current.id, 1_234).await.unwrap());

    assert_eq!(
        status_of(&pool, 100).await,
        ("READY".into(), Some(1_234), 2)
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_recording_fails_after_the_last_attempt(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());
    repo.register(&segment(100, BROADCAST)).await.unwrap();

    for attempt in 1..=MAX_STORE_ATTEMPTS {
        let claimed = repo.claim(LEASE).await.unwrap().expect("due again");
        assert_eq!(claimed.attempts, attempt);
        let retry = match StoreRetry::after_attempt(claimed.attempts) {
            // Due at once, so the test need not wait out the backoff.
            StoreRetry::After(_) => StoreRetry::After(Duration::ZERO),
            give_up => give_up,
        };
        assert!(
            repo.release_failed(claimed.id, claimed.attempts, retry)
                .await
                .unwrap()
        );
    }

    assert_eq!(
        status_of(&pool, 100).await,
        ("FAILED".into(), None, MAX_STORE_ATTEMPTS as i32)
    );
    assert!(repo.claim(LEASE).await.unwrap().is_none());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_failed_attempt_waits_out_its_backoff(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());
    repo.register(&segment(100, BROADCAST)).await.unwrap();

    let claimed = repo.claim(LEASE).await.unwrap().unwrap();
    let wait = StoreRetry::after_attempt(claimed.attempts);
    assert!(
        repo.release_failed(claimed.id, claimed.attempts, wait)
            .await
            .unwrap()
    );

    assert!(repo.claim(LEASE).await.unwrap().is_none());
    let waits: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM retry_at - now())::float8 FROM recordings WHERE id = 100",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!((25.0..=30.0).contains(&waits), "{waits}");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn recordings_of_a_deleted_broadcast_cannot_be_read(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());
    let query = PgRecordingQuery::new(pool.clone());
    for (recording, broadcast) in [(100, BROADCAST), (101, OTHER_BROADCAST), (102, BROADCAST)] {
        repo.register(&segment(recording, broadcast)).await.unwrap();
    }
    repo.mark_stored(id(102), 77).await.unwrap();

    let ids = |views: Vec<application::ports::recording::RecordingView>| {
        views.into_iter().map(|v| v.id.as_i64()).collect::<Vec<_>>()
    };
    let all = query.list(RecordingFilter::All, None, 10).await.unwrap();
    assert_eq!(ids(all.clone()), [102, 101, 100], "newest first");
    let newest = &all[0];
    assert_eq!(newest.broadcast_title.as_str(), "first");
    assert_eq!(newest.owner_name.as_str(), "owner");
    assert_eq!(newest.status, RecordingStatus::Ready);
    assert_eq!(newest.size_bytes, Some(77));
    assert_eq!(newest.duration, Duration::from_millis(5_940));
    assert_eq!(newest.key, RecordingKey::new(id(BROADCAST), id(102)));
    assert_eq!(
        ids(query
            .list(RecordingFilter::Broadcast(id(BROADCAST)), Some(id(102)), 10)
            .await
            .unwrap()),
        [100]
    );
    assert_eq!(
        ids(query.list(RecordingFilter::All, None, 1).await.unwrap()),
        [102]
    );
    assert!(query.get(id(101)).await.unwrap().is_some());

    delete_broadcast(&pool, OTHER_BROADCAST).await;

    assert!(query.get(id(101)).await.unwrap().is_none());
    assert_eq!(
        ids(query.list(RecordingFilter::All, None, 10).await.unwrap()),
        [102, 100]
    );
    assert!(
        query
            .list(RecordingFilter::Broadcast(id(OTHER_BROADCAST)), None, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_stale_storer_that_succeeds_still_stores_the_recording(pool: PgPool) {
    seed(&pool).await;
    let repo = PgRecordingRepository::new(pool.clone());
    repo.register(&segment(100, BROADCAST)).await.unwrap();

    let stale = repo.claim(Duration::ZERO).await.unwrap().unwrap();
    let current = repo.claim(LEASE).await.unwrap().expect("the lease ran out");
    assert!(
        repo.mark_stored(stale.id, 1_234).await.unwrap(),
        "the same key holds the same bytes, so the stale success counts"
    );
    assert!(
        !repo
            .release_failed(current.id, current.attempts, StoreRetry::GiveUp)
            .await
            .unwrap(),
        "a later failure cannot undo a stored recording"
    );

    assert_eq!(
        status_of(&pool, 100).await,
        ("READY".into(), Some(1_234), 2)
    );
}
