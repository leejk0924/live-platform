//! Runs the reaper in the background of the server process.

use std::{sync::Arc, time::Duration};

use application::{
    ports::PortError,
    reaper::{REAP_INTERVAL, Reaped, Reaper},
};
use domain::broadcast::BroadcastId;

use crate::audit::{AuditEvent, audit};
use tokio::{
    sync::oneshot,
    task::{JoinError, JoinHandle},
    time::MissedTickBehavior,
};

/// How long stopping waits for the pass in progress before dropping it, so
/// a slow media server cannot hold up the rest of the shutdown past the
/// container's stop timeout. A pass dropped before its commit is rolled
/// back; one dropped while kicking after the commit leaves streams that the
/// next pass finds and kicks again.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

pub struct ReaperHandle {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl ReaperHandle {
    /// Stops after the pass in progress, if any, has finished, or after
    /// [`SHUTDOWN_WAIT`].
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
                biased;
                _ = &mut stopped => return,
                _ = ticks.tick() => {}
            }
            // Its own task, so a panic ends this pass and not the loop.
            let reaper = Arc::clone(&reaper);
            let mut pass = tokio::spawn(async move { reaper.tick().await });
            let finished = tokio::select! {
                finished = &mut pass => finished,
                _ = &mut stopped => {
                    match tokio::time::timeout(SHUTDOWN_WAIT, &mut pass).await {
                        Ok(finished) => report_pass(finished),
                        Err(_) => {
                            pass.abort();
                            tracing::warn!("reaper pass dropped at shutdown");
                        }
                    }
                    return;
                }
            };
            report_pass(finished);
        }
    });
    ReaperHandle { stop, task }
}

fn report_pass(finished: Result<Result<Option<Reaped>, PortError>, JoinError>) {
    let reaped = match finished {
        Ok(Ok(Some(reaped))) => reaped,
        Ok(Ok(None)) => return,
        Err(err) => {
            tracing::error!(%err, "reaper pass panicked");
            return;
        }
        Ok(Err(err)) => {
            tracing::error!(%err, "reaper pass failed");
            return;
        }
    };
    if let Some(err) = &reaped.probe_failure {
        tracing::warn!(%err, "reaper could not list the streams on air; nothing reconciled");
    }
    for (broadcast, err) in &reaped.kick_failures {
        audit!(
            warn,
            AuditEvent::KickFailed,
            %err,
            %broadcast,
            by = "reaper",
            "publisher was not disconnected; the next pass retries"
        );
    }
    for err in &reaped.housekeeping_failures {
        tracing::warn!(%err, "reaper housekeeping step failed; the next pass retries");
    }
    let failed = |id: &BroadcastId| reaped.kick_failures.iter().any(|(f, _)| f == id);
    for broadcast in reaped.not_active.iter().filter(|id| !failed(id)) {
        audit!(
            warn,
            AuditEvent::PublisherKicked,
            %broadcast,
            by = "reaper",
            "publisher disconnect sent for a broadcast that is not active"
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
    if reaped.forgotten_sessions > 0 {
        tracing::info!(
            sessions = reaped.forgotten_sessions,
            "reaper deleted expired sessions"
        );
    }
    if !reaped.demo_reset.is_empty() {
        tracing::info!(
            broadcasts = %joined(&reaped.demo_reset),
            "reaper reset demo broadcasts from before today"
        );
    }
}

fn joined(ids: &[BroadcastId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use std::{io, sync::Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct LogBuffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for LogBuffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn id(raw: i64) -> BroadcastId {
        BroadcastId::try_from(raw).unwrap()
    }

    #[test]
    fn report_pass_audits_each_kicked_broadcast_except_failed_ones() {
        let logs = LogBuffer::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        let (kicked, failed) = (id(1_001), id(1_002));
        let reaped = Reaped {
            not_active: vec![kicked, failed],
            kick_failures: vec![(failed, PortError("media server down".into()))],
            ..Reaped::default()
        };

        tracing::subscriber::with_default(subscriber, || report_pass(Ok(Ok(Some(reaped)))));

        let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        let audits = |event: &str| -> Vec<&str> {
            let event = format!(r#""event":"{event}""#);
            logged
                .lines()
                .filter(|line| line.contains(r#""target":"audit""#) && line.contains(&event))
                .collect()
        };
        let [kick] = audits("publisher_kicked")[..] else {
            panic!("one publisher_kicked audit expected in {logged}");
        };
        assert!(
            kick.contains(&format!(r#""broadcast":"{kicked}""#)),
            "{kick}"
        );
        assert!(kick.contains(r#""by":"reaper""#), "{kick}");
        let [failure] = audits("kick_failed")[..] else {
            panic!("one kick_failed audit expected in {logged}");
        };
        assert!(
            failure.contains(&format!(r#""broadcast":"{failed}""#)),
            "{failure}"
        );
        assert!(failure.contains(r#""by":"reaper""#), "{failure}");
    }
}
