//! The reaper's passes against Postgres, with the media server faked. Times
//! are set in the database relative to its own `now()`, so no clock is
//! mocked.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use application::{
    ports::{
        PortError,
        media::{IngestRouter, StreamPath, StreamProbe, StreamTerminator},
    },
    reaper::{Reaped, Reaper},
};
use async_trait::async_trait;
use domain::broadcast::BroadcastId;
use infrastructure::{media::MediaMtxIngestRouter, persistence::PgReaperStore};
use sqlx::PgPool;

/// Says these broadcasts are on air, or fails, after `delay`.
struct Probe {
    on_air: Mutex<Option<Vec<i64>>>,
    delay: Duration,
}

impl Probe {
    fn on_air(ids: &[i64]) -> Self {
        Self {
            on_air: Mutex::new(Some(ids.to_vec())),
            delay: Duration::ZERO,
        }
    }

    fn unreachable() -> Self {
        Self {
            on_air: Mutex::new(None),
            delay: Duration::ZERO,
        }
    }
}

#[async_trait]
impl StreamProbe for Probe {
    async fn live_paths(&self) -> Result<Vec<StreamPath>, PortError> {
        tokio::time::sleep(self.delay).await;
        let on_air = self.on_air.lock().unwrap().clone();
        let ids = on_air.ok_or_else(|| PortError("mediamtx api: connection refused".into()))?;
        Ok(ids
            .into_iter()
            .map(|id| {
                MediaMtxIngestRouter
                    .route(BroadcastId::try_from(id).unwrap())
                    .path
            })
            .collect())
    }
}

#[derive(Default)]
struct Kicks(Mutex<Vec<String>>);

#[async_trait]
impl StreamTerminator for Kicks {
    async fn kick(&self, path: &StreamPath) -> Result<(), PortError> {
        self.0.lock().unwrap().push(path.as_str().to_owned());
        Ok(())
    }
}

fn reaper(pool: &PgPool, probe: Probe, kicks: Arc<Kicks>) -> Reaper {
    Reaper {
        store: Arc::new(PgReaperStore::new(pool.clone())),
        probe: Arc::new(probe),
        ingest: Arc::new(MediaMtxIngestRouter),
        terminator: kicks,
    }
}

async fn owners(pool: &PgPool, count: i64) {
    for id in 1..=count {
        sqlx::query("INSERT INTO users (id, username, is_demo) VALUES ($1, $2, true)")
            .bind(id)
            .bind(format!("owner{id}"))
            .execute(pool)
            .await
            .unwrap();
    }
}

/// A broadcast of its own owner in `status`, last changed `changed_secs_ago`
/// and, when live or interrupted, in that status since then.
async fn broadcast(pool: &PgPool, id: i64, status: &str, changed_secs_ago: f64) {
    sqlx::query(
        "INSERT INTO broadcasts \
         (id, owner_id, title, status, updated_at, interrupted_at, live_since) \
         VALUES ($1, $1, 't', $2, now() - make_interval(secs => $3), \
                 CASE WHEN $2 = 'INTERRUPTED' THEN now() - make_interval(secs => $3) END, \
                 CASE WHEN $2 = 'LIVE' THEN now() - make_interval(secs => $3) END)",
    )
    .bind(id)
    .bind(status)
    .bind(changed_secs_ago)
    .execute(pool)
    .await
    .unwrap();
}

/// `(status, version, interrupted_at set, ended_at set)`.
async fn stored(pool: &PgPool, id: i64) -> (String, i32, bool, bool) {
    sqlx::query_as(
        "SELECT status, version, interrupted_at IS NOT NULL, ended_at IS NOT NULL \
         FROM broadcasts WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn ids(raw: &[i64]) -> Vec<BroadcastId> {
    raw.iter()
        .map(|&id| BroadcastId::try_from(id).unwrap())
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn interrupted_broadcasts_end_after_the_grace_and_are_kicked(pool: PgPool) {
    owners(&pool, 3).await;
    broadcast(&pool, 1, "INTERRUPTED", 31.0).await;
    broadcast(&pool, 2, "INTERRUPTED", 25.0).await;
    broadcast(&pool, 3, "LIVE", 300.0).await;
    let kicks = Arc::new(Kicks::default());

    let reaped = reaper(&pool, Probe::on_air(&[3]), kicks.clone())
        .tick()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reaped.expired, ids(&[1]));
    assert_eq!(stored(&pool, 1).await, ("ENDED".into(), 2, false, true));
    assert_eq!(
        stored(&pool, 2).await,
        ("INTERRUPTED".into(), 1, true, false),
        "still within the grace"
    );
    assert_eq!(stored(&pool, 3).await, ("LIVE".into(), 1, false, false));
    assert_eq!(*kicks.0.lock().unwrap(), ["live/1"]);

    let again = reaper(&pool, Probe::on_air(&[3]), kicks.clone())
        .tick()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        again,
        Reaped::default(),
        "a second pass finds nothing to do"
    );
    assert_eq!(kicks.0.lock().unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn missed_reports_are_repaired_from_what_is_on_air(pool: PgPool) {
    owners(&pool, 5).await;
    broadcast(&pool, 1, "LIVE", 31.0).await;
    broadcast(&pool, 2, "LIVE", 5.0).await;
    broadcast(&pool, 3, "INTERRUPTED", 10.0).await;
    broadcast(&pool, 4, "INTERRUPTED", 45.0).await;
    broadcast(&pool, 5, "ENDED", 100.0).await;
    let kicks = Arc::new(Kicks::default());

    let reaped = reaper(&pool, Probe::on_air(&[3, 4, 5]), kicks.clone())
        .tick()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        reaped.reconciled.interrupted,
        ids(&[1]),
        "live without a stream for longer than the grace"
    );
    assert_eq!(
        stored(&pool, 1).await,
        ("INTERRUPTED".into(), 2, true, false)
    );
    assert_eq!(
        stored(&pool, 2).await,
        ("LIVE".into(), 1, false, false),
        "just started; its publisher may still be connecting"
    );
    let mut resumed = reaped.reconciled.resumed.clone();
    resumed.sort_by_key(|id| id.as_i64());
    assert_eq!(resumed, ids(&[3, 4]), "on air again, even past the grace");
    assert_eq!(stored(&pool, 3).await, ("LIVE".into(), 2, false, false));
    assert_eq!(stored(&pool, 4).await, ("LIVE".into(), 2, false, false));
    assert!(reaped.expired.is_empty());
    assert_eq!(stored(&pool, 5).await, ("ENDED".into(), 1, false, false));
    assert_eq!(reaped.not_active, ids(&[5]), "ended, yet still on air");
    assert_eq!(*kicks.0.lock().unwrap(), ["live/5"]);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_unreachable_media_server_interrupts_nothing_but_expiry_goes_on(pool: PgPool) {
    owners(&pool, 2).await;
    broadcast(&pool, 1, "LIVE", 300.0).await;
    broadcast(&pool, 2, "INTERRUPTED", 31.0).await;
    let kicks = Arc::new(Kicks::default());

    let reaped = reaper(&pool, Probe::unreachable(), kicks.clone())
        .tick()
        .await
        .unwrap()
        .unwrap();

    assert!(reaped.probe_failure.is_some());
    assert_eq!(reaped.reconciled, Default::default());
    assert_eq!(stored(&pool, 1).await, ("LIVE".into(), 1, false, false));
    assert_eq!(reaped.expired, ids(&[2]));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn spent_tickets_are_forgotten_after_they_can_no_longer_be_shown(pool: PgPool) {
    owners(&pool, 1).await;
    broadcast(&pool, 1, "LIVE", 0.0).await;
    for (jti, expired_secs_ago) in [("old", 400.0), ("recent", 30.0), ("valid", -30.0)] {
        sqlx::query(
            "INSERT INTO publish_token_uses (jti, broadcast_id, expires_at) \
             VALUES ($1, 1, now() - make_interval(secs => $2))",
        )
        .bind(jti)
        .bind(expired_secs_ago)
        .execute(&pool)
        .await
        .unwrap();
    }

    let reaped = reaper(&pool, Probe::on_air(&[1]), Arc::default())
        .tick()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reaped.forgotten_tickets, 1);
    let left: Vec<String> = sqlx::query_scalar("SELECT jti FROM publish_token_uses ORDER BY jti")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, ["recent", "valid"]);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn two_reapers_never_run_a_pass_at_the_same_time(pool: PgPool) {
    owners(&pool, 1).await;
    broadcast(&pool, 1, "INTERRUPTED", 31.0).await;
    let kicks = Arc::new(Kicks::default());
    let slow = reaper(
        &pool,
        Probe {
            on_air: Mutex::new(Some(Vec::new())),
            delay: Duration::from_millis(500),
        },
        kicks.clone(),
    );
    let other = reaper(&pool, Probe::on_air(&[]), kicks.clone());

    let (first, second) = tokio::join!(slow.tick(), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        other.tick().await
    });

    let first = first.unwrap().expect("the first pass ran");
    assert_eq!(first.expired, ids(&[1]));
    assert_eq!(second.unwrap(), None, "the second found the lock held");
    assert_eq!(*kicks.0.lock().unwrap(), ["live/1"], "kicked once");

    let after = other.tick().await.unwrap();
    assert_eq!(after, Some(Reaped::default()), "the lock was released");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_edit_does_not_postpone_the_interrupt_of_a_live_broadcast_without_a_stream(
    pool: PgPool,
) {
    owners(&pool, 1).await;
    broadcast(&pool, 1, "LIVE", 60.0).await;
    sqlx::query("UPDATE broadcasts SET title = 'edited', updated_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();

    let reaped = reaper(&pool, Probe::on_air(&[]), Arc::default())
        .tick()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reaped.reconciled.interrupted, ids(&[1]));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn streams_of_broadcasts_that_are_not_on_air_are_kicked_every_pass(pool: PgPool) {
    owners(&pool, 3).await;
    broadcast(&pool, 1, "LIVE", 60.0).await;
    broadcast(&pool, 2, "ENDED", 60.0).await;
    broadcast(&pool, 3, "ENDED", 60.0).await;
    sqlx::query("UPDATE broadcasts SET deleted_at = now() WHERE id = 3")
        .execute(&pool)
        .await
        .unwrap();
    let unknown = 99;
    let kicks = Arc::new(Kicks::default());

    for pass in 1..=2 {
        let mut reaped = reaper(&pool, Probe::on_air(&[1, 2, 3, unknown]), kicks.clone())
            .tick()
            .await
            .unwrap()
            .unwrap();
        reaped.not_active.sort_by_key(|id| id.as_i64());
        assert_eq!(reaped.not_active, ids(&[2, 3, unknown]), "pass {pass}");
        let mut kicked = kicks.0.lock().unwrap().clone();
        kicked.sort();
        assert_eq!(kicked, ["live/2", "live/3", "live/99"], "pass {pass}");
        kicks.0.lock().unwrap().clear();
    }
    assert_eq!(stored(&pool, 2).await, ("ENDED".into(), 1, false, false));
}
