//! Snowflake id issuing backed by a leased worker id.

use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use application::ports::id::{
    IdGenerationError, IdGenerator, LeaseError, WorkerIdAllocator, WorkerLease,
};
use domain::id::{EPOCH_UNIX_MS, NextIdError, SnowflakeId, SnowflakeSequencer};
use thiserror::Error;
use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
    time::{MissedTickBehavior, interval_at},
};

/// A previous holder of the worker id ran this far ahead of our clock at most
/// before we refuse to start; beyond it the host clock is likely wrong.
pub const MAX_STARTUP_BACKWARD: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum StartError {
    #[error(transparent)]
    Lease(#[from] LeaseError),
    #[error("clock is {behind_ms} ms behind the worker id's last use")]
    ClockBehind { behind_ms: u64 },
    #[error("wall clock is before the snowflake epoch")]
    ClockBeforeEpoch,
}

/// Milliseconds since the snowflake epoch that never go backwards: the wall
/// clock is read once at start and then advanced by `Instant`.
struct MonotonicClock {
    base_ms: u64,
    started: Instant,
}

impl MonotonicClock {
    fn start() -> Result<Self, StartError> {
        Self::at_unix_ms(wall_unix_ms())
    }

    fn at_unix_ms(unix_ms: u64) -> Result<Self, StartError> {
        let base_ms = unix_ms
            .checked_sub(EPOCH_UNIX_MS)
            .ok_or(StartError::ClockBeforeEpoch)?;
        Ok(Self {
            base_ms,
            started: Instant::now(),
        })
    }

    fn now_ms(&self) -> u64 {
        self.base_ms + self.started.elapsed().as_millis() as u64
    }
}

fn wall_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Stop issuing a third of the TTL before the database could hand the worker
/// id to another instance, measured from when the renewal was sent.
fn lease_deadline(sent_at_ms: u64, ttl: Duration) -> u64 {
    let ttl_ms = ttl.as_millis() as u64;
    sent_at_ms + ttl_ms - ttl_ms / 3
}

pub struct SnowflakeIdGenerator {
    clock: MonotonicClock,
    sequencer: Mutex<SnowflakeSequencer>,
    valid_until_ms: AtomicU64,
}

impl SnowflakeIdGenerator {
    /// Leases a worker id, waits until the clock has passed the id's last use,
    /// and starts renewing the lease every `ttl / 3`.
    pub async fn start(
        allocator: Arc<dyn WorkerIdAllocator>,
        ttl: Duration,
    ) -> Result<(Arc<Self>, HeartbeatHandle), StartError> {
        let clock = MonotonicClock::start()?;
        let requested_at = clock.now_ms();
        let lease = allocator.acquire().await?;

        let behind_ms = lease.floor_ms.saturating_sub(clock.now_ms());
        if behind_ms > MAX_STARTUP_BACKWARD.as_millis() as u64 {
            release_unused(allocator.as_ref(), &lease).await;
            return Err(StartError::ClockBehind { behind_ms });
        }

        // Store the bound before issuing anything, so a crash before the first
        // heartbeat still leaves the next holder a floor above our ids.
        let valid_until = lease_deadline(requested_at, ttl);
        if let Err(err) = allocator.renew(&lease, valid_until).await {
            release_unused(allocator.as_ref(), &lease).await;
            return Err(err.into());
        }

        let generator = Arc::new(Self {
            clock,
            sequencer: Mutex::new(SnowflakeSequencer::new(lease.worker, lease.floor_ms)),
            valid_until_ms: AtomicU64::new(valid_until),
        });
        tracing::info!(
            worker = lease.worker.get(),
            floor_ms = lease.floor_ms,
            "worker id leased"
        );
        let heartbeat = HeartbeatHandle::spawn(Arc::clone(&generator), allocator, lease, ttl);
        if behind_ms > 0 {
            tokio::time::sleep(Duration::from_millis(behind_ms + 1)).await;
        }
        Ok((generator, heartbeat))
    }

    fn last_ms(&self) -> u64 {
        self.lock().last_ms()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SnowflakeSequencer> {
        // The sequencer never panics mid-update, so a poisoned state is still consistent.
        self.sequencer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Gives back a lease no id was issued under, leaving its floor as it was.
async fn release_unused(allocator: &dyn WorkerIdAllocator, lease: &WorkerLease) {
    if let Err(err) = allocator.release(lease, lease.floor_ms).await {
        tracing::warn!(%err, "could not release worker id lease after refusing to start");
    }
}

impl IdGenerator for SnowflakeIdGenerator {
    fn next_id(&self) -> Result<SnowflakeId, IdGenerationError> {
        let mut sequencer = self.lock();
        loop {
            let now = self.clock.now_ms();
            if now >= self.valid_until_ms.load(Ordering::Acquire) {
                return Err(IdGenerationError::LeaseExpired);
            }
            match sequencer.next(now) {
                Ok(id) => return Ok(id),
                // The next millisecond is less than 1 ms away.
                Err(NextIdError::SequenceExhausted { .. }) => std::hint::spin_loop(),
                Err(NextIdError::ClockBehind { .. }) => {
                    return Err(IdGenerationError::ClockBehind);
                }
                Err(NextIdError::TimestampOverflow) => {
                    return Err(IdGenerationError::TimestampOverflow);
                }
            }
        }
    }
}

/// Owns the lease renewal task. Call [`HeartbeatHandle::shutdown`] to stop it
/// and give the worker id back.
pub struct HeartbeatHandle {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
    lost: watch::Receiver<bool>,
    generator: Arc<SnowflakeIdGenerator>,
    allocator: Arc<dyn WorkerIdAllocator>,
    lease: WorkerLease,
}

impl HeartbeatHandle {
    fn spawn(
        generator: Arc<SnowflakeIdGenerator>,
        allocator: Arc<dyn WorkerIdAllocator>,
        lease: WorkerLease,
        ttl: Duration,
    ) -> Self {
        let (stop, stopped) = oneshot::channel();
        let (lost_tx, lost) = watch::channel(false);
        let task = tokio::spawn(heartbeat(
            Arc::clone(&generator),
            Arc::clone(&allocator),
            lease.clone(),
            ttl,
            stopped,
            lost_tx,
        ));
        Self {
            stop,
            task,
            lost,
            generator,
            allocator,
            lease,
        }
    }

    /// Resolves once the lease is lost or the renewal task has ended without
    /// being stopped; either way this process can no longer issue ids.
    pub fn lease_lost(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut lost = self.lost.clone();
        async move {
            let _ = lost.wait_for(|lost| *lost).await;
        }
    }

    /// Stops issuing ids, then releases the lease with the last used ms so
    /// the next holder starts after it.
    pub async fn shutdown(self) -> Result<(), LeaseError> {
        let _ = self.stop.send(());
        let _ = self.task.await;
        self.generator.valid_until_ms.store(0, Ordering::Release);
        // Taking the lock waits out an in-flight next_id, so last_ms is final.
        let last_ms = self.generator.last_ms();
        self.allocator.release(&self.lease, last_ms).await
    }
}

async fn heartbeat(
    generator: Arc<SnowflakeIdGenerator>,
    allocator: Arc<dyn WorkerIdAllocator>,
    lease: WorkerLease,
    ttl: Duration,
    mut stopped: oneshot::Receiver<()>,
    lost: watch::Sender<bool>,
) {
    let period = (ttl / 3).max(Duration::from_millis(1));
    let mut ticks = interval_at(tokio::time::Instant::now() + period, period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = &mut stopped => return,
            _ = ticks.tick() => {}
        }
        // Every id issued until the next renewal is below this deadline, so
        // storing it keeps the database's last_ms ahead of us if we crash.
        let valid_until = lease_deadline(generator.clock.now_ms(), ttl);
        let renewed = tokio::select! {
            _ = &mut stopped => return,
            renewed = allocator.renew(&lease, valid_until) => renewed,
        };
        match renewed {
            Ok(()) => generator
                .valid_until_ms
                .store(valid_until, Ordering::Release),
            Err(LeaseError::Lost) => {
                generator.valid_until_ms.store(0, Ordering::Release);
                tracing::error!(
                    worker = lease.worker.get(),
                    "worker id lease lost, id generation stopped"
                );
                lost.send_replace(true);
                return;
            }
            Err(err) => tracing::warn!(%err, "worker id lease renewal failed, will retry"),
        }
    }
}

#[cfg(test)]
mod tests {
    use application::ports::id::LeaseHolder;
    use async_trait::async_trait;
    use domain::id::WorkerId;

    use super::*;

    struct FakeAllocator {
        floor_ms: u64,
        renew: Result<(), LeaseError>,
        /// Renewals before this many calls succeed regardless of `renew`.
        renew_fails_from: usize,
        renewed: Mutex<Vec<u64>>,
        released: Mutex<Option<u64>>,
    }

    impl FakeAllocator {
        /// `renew` answers every heartbeat; the renewal made at startup succeeds.
        fn new(floor_ms: u64, renew: Result<(), LeaseError>) -> Arc<Self> {
            Self::failing_from(floor_ms, renew, 1)
        }

        fn failing_from(
            floor_ms: u64,
            renew: Result<(), LeaseError>,
            renew_fails_from: usize,
        ) -> Arc<Self> {
            Arc::new(Self {
                floor_ms,
                renew,
                renew_fails_from,
                renewed: Mutex::new(Vec::new()),
                released: Mutex::new(None),
            })
        }

        fn renewed(&self) -> Vec<u64> {
            self.renewed.lock().unwrap().clone()
        }

        fn released(&self) -> Option<u64> {
            *self.released.lock().unwrap()
        }
    }

    #[async_trait]
    impl WorkerIdAllocator for FakeAllocator {
        async fn acquire(&self) -> Result<WorkerLease, LeaseError> {
            Ok(WorkerLease {
                worker: WorkerId::try_from(5).unwrap(),
                holder: LeaseHolder::new("fake".into()),
                floor_ms: self.floor_ms,
            })
        }

        async fn renew(&self, _: &WorkerLease, last_ms: u64) -> Result<(), LeaseError> {
            let mut renewed = self.renewed.lock().unwrap();
            if renewed.len() >= self.renew_fails_from {
                self.renew.clone()?;
            }
            renewed.push(last_ms);
            Ok(())
        }

        async fn release(&self, _: &WorkerLease, last_ms: u64) -> Result<(), LeaseError> {
            *self.released.lock().unwrap() = Some(last_ms);
            Ok(())
        }
    }

    #[tokio::test]
    async fn issues_increasing_ids_past_sequence_exhaustion() {
        let allocator = FakeAllocator::new(0, Ok(()));
        let (generator, heartbeat) =
            SnowflakeIdGenerator::start(allocator, Duration::from_secs(30))
                .await
                .unwrap();

        let ids: Vec<_> = (0..20_000).map(|_| generator.next_id().unwrap()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert!(ids.iter().all(|id| id.worker().get() == 5));
        heartbeat.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn startup_records_an_upper_bound_before_the_first_id() {
        let allocator = FakeAllocator::new(0, Ok(()));
        let (generator, heartbeat) =
            SnowflakeIdGenerator::start(allocator.clone(), Duration::from_secs(30))
                .await
                .unwrap();

        let recorded = allocator.renewed();
        let id = generator.next_id().unwrap();
        assert_eq!(recorded.len(), 1, "one renewal before any id: {recorded:?}");
        assert!(id.timestamp_ms() < recorded[0], "{recorded:?} vs {id:?}");
        heartbeat.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn start_fails_and_releases_when_the_bound_cannot_be_recorded() {
        let allocator =
            FakeAllocator::failing_from(7, Err(LeaseError::Unavailable("down".into())), 0);

        let result = SnowflakeIdGenerator::start(allocator.clone(), Duration::from_secs(30)).await;
        assert!(matches!(
            result,
            Err(StartError::Lease(LeaseError::Unavailable(_)))
        ));
        assert_eq!(allocator.released(), Some(7));
    }

    #[tokio::test]
    async fn renewals_keep_ids_flowing_and_bound_every_issued_id() {
        let allocator = FakeAllocator::new(0, Ok(()));
        let (generator, heartbeat) =
            SnowflakeIdGenerator::start(allocator.clone(), Duration::from_millis(90))
                .await
                .unwrap();

        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(300) {
            let id = generator.next_id().expect("renewed lease keeps issuing");
            let recorded = *allocator.renewed().last().unwrap();
            assert!(
                id.timestamp_ms() < recorded,
                "the stored bound {recorded} must exceed issued {}",
                id.timestamp_ms()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(allocator.renewed().len() >= 5, "{:?}", allocator.renewed());
        heartbeat.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn ids_stop_once_the_lease_cannot_be_renewed() {
        let allocator = FakeAllocator::new(0, Err(LeaseError::Unavailable("down".into())));
        let (generator, heartbeat) =
            SnowflakeIdGenerator::start(allocator, Duration::from_millis(90))
                .await
                .unwrap();
        assert!(generator.next_id().is_ok());

        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(generator.next_id(), Err(IdGenerationError::LeaseExpired));
        heartbeat.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn lost_lease_is_announced_to_the_owner() {
        let allocator = FakeAllocator::new(0, Err(LeaseError::Lost));
        let (generator, heartbeat) =
            SnowflakeIdGenerator::start(allocator, Duration::from_millis(300))
                .await
                .unwrap();

        tokio::time::timeout(Duration::from_secs(2), heartbeat.lease_lost())
            .await
            .expect("lease loss resolves lease_lost");
        assert_eq!(generator.next_id(), Err(IdGenerationError::LeaseExpired));
        heartbeat.shutdown().await.unwrap();
    }

    #[test]
    fn a_wall_clock_before_the_snowflake_epoch_is_refused() {
        assert!(matches!(
            MonotonicClock::at_unix_ms(EPOCH_UNIX_MS - 1),
            Err(StartError::ClockBeforeEpoch)
        ));
        assert!(matches!(
            MonotonicClock::at_unix_ms(0),
            Err(StartError::ClockBeforeEpoch)
        ));
        let clock = MonotonicClock::at_unix_ms(EPOCH_UNIX_MS + 5).unwrap();
        assert!(clock.now_ms() >= 5);
    }

    #[tokio::test]
    async fn start_refuses_a_large_clock_regression() {
        let floor_ms = (wall_unix_ms() - EPOCH_UNIX_MS) + 60_000;
        let allocator = FakeAllocator::new(floor_ms, Ok(()));

        let result = SnowflakeIdGenerator::start(allocator.clone(), Duration::from_secs(30)).await;
        assert!(matches!(result, Err(StartError::ClockBehind { behind_ms }) if behind_ms > 50_000));
        assert_eq!(
            allocator.released(),
            Some(floor_ms),
            "the refused lease is given back"
        );
    }

    #[tokio::test]
    async fn start_waits_until_the_clock_passes_the_floor() {
        let floor_ms = (wall_unix_ms() - EPOCH_UNIX_MS) + 300;
        let allocator = FakeAllocator::new(floor_ms, Ok(()));

        let started = Instant::now();
        let (generator, heartbeat) =
            SnowflakeIdGenerator::start(allocator, Duration::from_secs(30))
                .await
                .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250));

        let id = generator.next_id().unwrap();
        assert!(id.timestamp_ms() > floor_ms);
        heartbeat.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_releases_with_last_used_ms_and_stops_ids() {
        let allocator = FakeAllocator::new(0, Ok(()));
        let (generator, heartbeat) =
            SnowflakeIdGenerator::start(allocator.clone(), Duration::from_secs(30))
                .await
                .unwrap();
        let id = generator.next_id().unwrap();

        heartbeat.shutdown().await.unwrap();
        assert_eq!(allocator.released(), Some(id.timestamp_ms()));
        assert_eq!(generator.next_id(), Err(IdGenerationError::LeaseExpired));
    }
}
