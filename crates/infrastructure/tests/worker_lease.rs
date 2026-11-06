use std::{collections::HashSet, sync::Arc, time::Duration};

use application::ports::id::{LeaseError, WorkerIdAllocator};
use infrastructure::persistence::PgWorkerIdAllocator;
use sqlx::PgPool;
use tokio::task::JoinSet;

fn allocator(pool: PgPool) -> PgWorkerIdAllocator {
    PgWorkerIdAllocator::new(pool, Duration::from_secs(30))
}

async fn last_ms(pool: &PgPool, worker: u16) -> i64 {
    sqlx::query_scalar("SELECT last_ms FROM worker_leases WHERE worker_id = $1")
        .bind(worker as i16)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn concurrent_acquires_get_distinct_workers(pool: PgPool) {
    let allocator = Arc::new(allocator(pool));
    let mut tasks = JoinSet::new();
    for _ in 0..32 {
        let allocator = Arc::clone(&allocator);
        tasks.spawn(async move { allocator.acquire().await });
    }

    let mut workers = HashSet::new();
    while let Some(result) = tasks.join_next().await {
        let lease = result.unwrap().unwrap();
        assert!(
            workers.insert(lease.worker),
            "worker {:?} was handed out twice",
            lease.worker
        );
    }
    assert_eq!(workers.len(), 32);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn renew_after_expiry_is_lost(pool: PgPool) {
    let allocator = allocator(pool.clone());
    let lease = allocator.acquire().await.unwrap();
    allocator.renew(&lease, 10).await.unwrap();

    sqlx::query(
        "UPDATE worker_leases SET expires_at = now() - interval '1 second' WHERE worker_id = $1",
    )
    .bind(lease.worker.get() as i16)
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(allocator.renew(&lease, 20).await, Err(LeaseError::Lost));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn renew_never_lowers_and_release_stores_the_exact_value(pool: PgPool) {
    let allocator = allocator(pool.clone());
    let lease = allocator.acquire().await.unwrap();
    assert_eq!(lease.floor_ms, 0);
    let worker = lease.worker.get();

    allocator.renew(&lease, 500).await.unwrap();
    allocator.renew(&lease, 300).await.unwrap();
    assert_eq!(last_ms(&pool, worker).await, 500, "renew never lowers");

    allocator.release(&lease, 400).await.unwrap();
    assert_eq!(
        last_ms(&pool, worker).await,
        400,
        "release replaces the renewal bound with the exact last use"
    );
    allocator.release(&lease, 900).await.unwrap();
    assert_eq!(
        last_ms(&pool, worker).await,
        400,
        "a released lease no longer writes"
    );

    sqlx::query("UPDATE worker_leases SET holder = 'other', expires_at = now() + interval '1 hour' WHERE worker_id <> $1")
        .bind(worker as i16)
        .execute(&pool)
        .await
        .unwrap();

    let again = allocator.acquire().await.unwrap();
    assert_eq!(again.worker, lease.worker);
    assert_eq!(again.floor_ms, 400);
    assert_ne!(again.holder, lease.holder);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn acquire_fails_when_every_worker_is_taken(pool: PgPool) {
    sqlx::query(
        "UPDATE worker_leases SET holder = 'other', expires_at = now() + interval '1 hour'",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(allocator(pool).acquire().await, Err(LeaseError::Exhausted));
}
