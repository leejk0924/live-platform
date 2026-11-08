//! The reaper: ends broadcasts whose publisher did not come back within the
//! grace period, repairs statuses the media server's reports missed, and
//! deletes records nobody can use any more.

use std::{sync::Arc, time::Duration};

use domain::broadcast::{BroadcastId, INTERRUPT_GRACE};

use crate::ports::{
    PortError,
    broadcast::{ReaperStore, Reconciled},
    media::{IngestRouter, StreamProbe, StreamTerminator},
};

/// How often a reaper looks. An interrupted broadcast ends at most this long
/// after its grace period.
pub const REAP_INTERVAL: Duration = Duration::from_secs(5);

/// What one pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reaped {
    pub reconciled: Reconciled,
    pub expired: Vec<BroadcastId>,
    /// On air although not active, such as a broadcast stopped while its
    /// publisher was being let in, or one whose kick failed before. Kicked on
    /// every pass that still sees them.
    pub not_active: Vec<BroadcastId>,
    pub forgotten_tickets: u64,
    pub forgotten_sessions: u64,
    /// Demo broadcasts from before today, deleted so the demo accounts start
    /// each day empty.
    pub demo_reset: Vec<BroadcastId>,
    /// The media server could not be asked, so nothing was reconciled. A
    /// pass never interrupts broadcasts because the media server is
    /// unreachable.
    pub probe_failure: Option<PortError>,
    /// Expired or not active broadcasts whose publisher could not be
    /// disconnected; the next pass tries again while they are on air.
    pub kick_failures: Vec<(BroadcastId, PortError)>,
    /// Housekeeping steps (forgetting tickets and sessions, resetting demo
    /// broadcasts) that failed. Each undoes only itself, so the status
    /// changes above are still stored and kicked.
    pub housekeeping_failures: Vec<PortError>,
}

pub struct Reaper {
    pub store: Arc<dyn ReaperStore>,
    pub probe: Arc<dyn StreamProbe>,
    pub ingest: Arc<dyn IngestRouter>,
    pub terminator: Arc<dyn StreamTerminator>,
}

impl Reaper {
    /// One pass, or `None` when another instance's pass is running. Reports
    /// go first so a publisher that came back is resumed rather than
    /// expired. Kicks run after the pass is stored, like a stop's.
    pub async fn tick(&self) -> Result<Option<Reaped>, PortError> {
        let Some(mut pass) = self.store.begin().await? else {
            return Ok(None);
        };
        let mut reaped = Reaped::default();
        match self.probe.live_paths().await {
            Ok(paths) => {
                let on_air: Vec<BroadcastId> = paths
                    .iter()
                    .filter_map(|path| self.ingest.broadcast_of(path))
                    .collect();
                reaped.reconciled = pass.reconcile(&on_air, INTERRUPT_GRACE).await?;
                reaped.not_active = pass.not_active(&on_air).await?;
            }
            Err(err) => reaped.probe_failure = Some(err),
        }
        reaped.expired = pass.expire(INTERRUPT_GRACE).await?;
        let failures = &mut reaped.housekeeping_failures;
        reaped.forgotten_tickets = settle(pass.forget_spent_tickets().await, failures);
        reaped.forgotten_sessions = settle(pass.forget_expired_sessions().await, failures);
        reaped.demo_reset = settle(pass.reset_demo_broadcasts().await, failures);
        pass.finish().await?;

        for &id in reaped.expired.iter().chain(&reaped.not_active) {
            if let Err(err) = self.terminator.kick(&self.ingest.route(id).path).await {
                reaped.kick_failures.push((id, err));
            }
        }
        Ok(Some(reaped))
    }
}

/// A housekeeping step's result, or nothing done when it failed. A later pass
/// retries it, so the status changes do not wait for it.
fn settle<T: Default>(step: Result<T, PortError>, failures: &mut Vec<PortError>) -> T {
    step.unwrap_or_else(|err| {
        failures.push(err);
        T::default()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use domain::id::SnowflakeId;

    use super::*;
    use crate::ports::{
        broadcast::ReapPass,
        media::{IngestEndpoint, StreamPath},
    };

    const EXPIRED: i64 = 7;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Housekeeping {
        Tickets,
        Sessions,
        Demo,
    }

    #[derive(Default)]
    struct Log {
        finished: bool,
        kicked: Vec<StreamPath>,
    }

    struct Store {
        failing: Housekeeping,
        log: Arc<Mutex<Log>>,
    }

    #[async_trait]
    impl ReaperStore for Store {
        async fn begin(&self) -> Result<Option<Box<dyn ReapPass>>, PortError> {
            Ok(Some(Box::new(Pass {
                failing: self.failing,
                log: self.log.clone(),
            })))
        }
    }

    /// Expires one broadcast and fails one housekeeping step.
    struct Pass {
        failing: Housekeeping,
        log: Arc<Mutex<Log>>,
    }

    impl Pass {
        fn step<T>(&self, step: Housekeeping, done: T) -> Result<T, PortError> {
            if self.failing == step {
                Err(PortError("housekeeping failed".into()))
            } else {
                Ok(done)
            }
        }
    }

    #[async_trait]
    impl ReapPass for Pass {
        async fn reconcile(
            &mut self,
            _: &[BroadcastId],
            _: Duration,
        ) -> Result<Reconciled, PortError> {
            Ok(Reconciled::default())
        }

        async fn not_active(&mut self, _: &[BroadcastId]) -> Result<Vec<BroadcastId>, PortError> {
            Ok(vec![])
        }

        async fn expire(&mut self, _: Duration) -> Result<Vec<BroadcastId>, PortError> {
            Ok(vec![SnowflakeId::try_from(EXPIRED).unwrap()])
        }

        async fn forget_spent_tickets(&mut self) -> Result<u64, PortError> {
            self.step(Housekeeping::Tickets, 0)
        }

        async fn forget_expired_sessions(&mut self) -> Result<u64, PortError> {
            self.step(Housekeeping::Sessions, 0)
        }

        async fn reset_demo_broadcasts(&mut self) -> Result<Vec<BroadcastId>, PortError> {
            self.step(Housekeeping::Demo, vec![])
        }

        async fn finish(self: Box<Self>) -> Result<(), PortError> {
            self.log.lock().unwrap().finished = true;
            Ok(())
        }
    }

    struct NothingOnAir;

    #[async_trait]
    impl StreamProbe for NothingOnAir {
        async fn live_paths(&self) -> Result<Vec<StreamPath>, PortError> {
            Ok(vec![])
        }
    }

    struct LivePath;

    impl IngestRouter for LivePath {
        fn route(&self, broadcast: BroadcastId) -> IngestEndpoint {
            IngestEndpoint {
                path: StreamPath::new(format!("live/{broadcast}")),
                whip_url: format!("/whip/{broadcast}"),
            }
        }

        fn broadcast_of(&self, _: &StreamPath) -> Option<BroadcastId> {
            None
        }
    }

    struct Kicks(Arc<Mutex<Log>>);

    #[async_trait]
    impl StreamTerminator for Kicks {
        async fn kick(&self, path: &StreamPath) -> Result<(), PortError> {
            self.0.lock().unwrap().kicked.push(path.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_failed_housekeeping_step_still_ends_expired_broadcasts() {
        for failing in [
            Housekeeping::Tickets,
            Housekeeping::Sessions,
            Housekeeping::Demo,
        ] {
            let log = Arc::new(Mutex::new(Log::default()));
            let reaper = Reaper {
                store: Arc::new(Store {
                    failing,
                    log: log.clone(),
                }),
                probe: Arc::new(NothingOnAir),
                ingest: Arc::new(LivePath),
                terminator: Arc::new(Kicks(log.clone())),
            };

            let reaped = reaper.tick().await.unwrap().unwrap();

            let expired = SnowflakeId::try_from(EXPIRED).unwrap();
            assert_eq!(reaped.expired, vec![expired]);
            assert_eq!(
                reaped.housekeeping_failures,
                vec![PortError("housekeeping failed".into())]
            );
            let log = log.lock().unwrap();
            assert!(log.finished, "the pass is stored");
            assert_eq!(log.kicked, vec![LivePath.route(expired).path]);
        }
    }
}
