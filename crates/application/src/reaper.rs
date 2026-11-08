//! The reaper: ends broadcasts whose publisher did not come back within the
//! grace period, and repairs statuses the media server's reports missed.

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
    pub forgotten_tickets: u64,
    /// The media server could not be asked, so nothing was reconciled. A
    /// pass never interrupts broadcasts because the media server is
    /// unreachable.
    pub probe_failure: Option<PortError>,
    /// Ended broadcasts whose publisher could not be disconnected.
    pub kick_failures: Vec<(BroadcastId, PortError)>,
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
            }
            Err(err) => reaped.probe_failure = Some(err),
        }
        reaped.expired = pass.expire(INTERRUPT_GRACE).await?;
        reaped.forgotten_tickets = pass.forget_spent_tickets().await?;
        pass.finish().await?;

        for &id in &reaped.expired {
            if let Err(err) = self.terminator.kick(&self.ingest.route(id).path).await {
                reaped.kick_failures.push((id, err));
            }
        }
        Ok(Some(reaped))
    }
}
