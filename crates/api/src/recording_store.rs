//! Runs the recording storer in the background of the server process.

use std::{sync::Arc, time::Duration};

use application::{
    ports::PortError,
    recording::{StoreOutcome, StoreRecordings},
};
use tokio::{
    sync::{Notify, oneshot},
    task::{JoinError, JoinHandle},
    time::MissedTickBehavior,
};

/// How often the storer looks for due recordings without being woken: the
/// retries whose backoff ran out and the claims whose lease expired.
pub const STORE_TICK: Duration = Duration::from_secs(5);

/// How long stopping waits for the recording in progress before dropping it.
/// A dropped one stays claimed until its lease runs out, then is stored
/// again from the start.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

pub struct RecordingStoreHandle {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl RecordingStoreHandle {
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        if let Err(err) = self.task.await {
            tracing::warn!(%err, "recording storer task failed");
        }
    }
}

/// Stores every due recording, then sleeps until `wake` fires, the next
/// tick, or shutdown. A failed or panicking attempt is logged and the loop
/// goes on; the stored state says what is left to do.
pub fn spawn(store: StoreRecordings, wake: Arc<Notify>) -> RecordingStoreHandle {
    let store = Arc::new(store);
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut ticks = tokio::time::interval(STORE_TICK);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            loop {
                // Its own task, so a panic ends this attempt and not the loop.
                let store = Arc::clone(&store);
                let mut attempt = tokio::spawn(async move { store.run_once().await });
                let finished = tokio::select! {
                    finished = &mut attempt => finished,
                    _ = &mut stopped => {
                        match tokio::time::timeout(SHUTDOWN_WAIT, &mut attempt).await {
                            Ok(finished) => {
                                report(finished);
                            }
                            Err(_) => {
                                attempt.abort();
                                tracing::warn!("recording store dropped at shutdown; retried after its lease");
                            }
                        }
                        return;
                    }
                };
                if !report(finished) {
                    break;
                }
            }
            tokio::select! {
                biased;
                _ = &mut stopped => return,
                () = wake.notified() => {}
                _ = ticks.tick() => {}
            }
        }
    });
    RecordingStoreHandle { stop, task }
}

/// Logs one attempt. Whether another recording may be due right away.
fn report(finished: Result<Result<Option<StoreOutcome>, PortError>, JoinError>) -> bool {
    let outcome = match finished {
        Ok(Ok(Some(outcome))) => outcome,
        Ok(Ok(None)) => return false,
        Ok(Err(err)) => {
            tracing::error!(%err, "recording store failed");
            return false;
        }
        Err(err) => {
            tracing::error!(%err, "recording store panicked");
            return false;
        }
    };
    match outcome {
        StoreOutcome::Stored {
            recording,
            size_bytes,
            discard,
        } => {
            tracing::info!(%recording, size_bytes, "recording stored");
            if let Err(err) = discard {
                tracing::warn!(%recording, %err, "stored recording left in the spool");
            }
        }
        StoreOutcome::Superseded(recording) => {
            tracing::info!(%recording, "recording was already stored by another attempt");
        }
        StoreOutcome::Retrying {
            recording,
            attempts,
            after,
            error,
        } => tracing::warn!(
            %recording,
            attempts,
            retry_in_secs = after.as_secs(),
            %error,
            "recording store failed; retrying"
        ),
        StoreOutcome::GaveUp { recording, error } => tracing::error!(
            %recording,
            %error,
            "recording store failed for the last time; recording marked failed"
        ),
    }
    true
}
