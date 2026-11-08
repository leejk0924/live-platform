//! Runs the reaper in the background of the server process.

use std::sync::Arc;

use application::{
    ports::PortError,
    reaper::{REAP_INTERVAL, Reaped, Reaper},
};
use domain::broadcast::BroadcastId;
use tokio::{sync::oneshot, task::JoinHandle, time::MissedTickBehavior};

pub struct ReaperHandle {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl ReaperHandle {
    /// Stops after the pass in progress, if any, has finished.
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        if let Err(err) = self.task.await {
            tracing::warn!(%err, "reaper task failed");
        }
    }
}

/// Ticks every [`REAP_INTERVAL`]. A failed or panicking pass is logged and
/// the next tick tries again; every pass derives its work from the stored
/// state, so a missed or repeated one changes nothing.
pub fn spawn(reaper: Reaper) -> ReaperHandle {
    let reaper = Arc::new(reaper);
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut ticks = tokio::time::interval(REAP_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = &mut stopped => return,
                _ = ticks.tick() => {}
            }
            // Its own task, so a panic ends this pass and not the loop.
            let reaper = Arc::clone(&reaper);
            match tokio::spawn(async move { reaper.tick().await }).await {
                Ok(result) => report(result),
                Err(err) => tracing::error!(%err, "reaper pass panicked"),
            }
        }
    });
    ReaperHandle { stop, task }
}

fn report(result: Result<Option<Reaped>, PortError>) {
    let reaped = match result {
        Ok(Some(reaped)) => reaped,
        Ok(None) => return,
        Err(err) => {
            tracing::error!(%err, "reaper pass failed");
            return;
        }
    };
    if let Some(err) = &reaped.probe_failure {
        tracing::warn!(%err, "reaper could not list the streams on air; nothing reconciled");
    }
    for (broadcast, err) in &reaped.kick_failures {
        tracing::warn!(%err, %broadcast, "publisher was not disconnected; the next pass retries");
    }
    if !reaped.not_active.is_empty() {
        tracing::warn!(
            broadcasts = %joined(&reaped.not_active),
            "disconnecting publishers of broadcasts that are not active"
        );
    }
    let changed = !reaped.expired.is_empty()
        || !reaped.reconciled.interrupted.is_empty()
        || !reaped.reconciled.resumed.is_empty();
    if changed {
        tracing::info!(
            expired = %joined(&reaped.expired),
            interrupted = %joined(&reaped.reconciled.interrupted),
            resumed = %joined(&reaped.reconciled.resumed),
            "reaper changed broadcast statuses"
        );
    }
}

fn joined(ids: &[BroadcastId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}
