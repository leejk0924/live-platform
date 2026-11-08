//! The background task around the reaper: it keeps ticking whatever a pass
//! does, and stopping it waits for the pass in progress. Time is paused, so
//! intervals pass instantly.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use api::reaper;
use application::{
    ports::{
        PortError,
        broadcast::{ReapPass, ReaperStore},
        media::{StreamPath, StreamProbe, StreamTerminator},
    },
    reaper::{REAP_INTERVAL, Reaper},
};
use async_trait::async_trait;
use infrastructure::media::MediaMtxIngestRouter;
use tokio::time::Instant;

/// What every pass does as it begins.
#[derive(Clone, Copy)]
enum Pass {
    /// Another instance holds the lock.
    Skipped,
    Fails,
    Takes(Duration),
}

struct Store {
    pass: Pass,
    begun: AtomicUsize,
    finished: AtomicUsize,
}

impl Store {
    fn new(pass: Pass) -> Arc<Self> {
        Arc::new(Self {
            pass,
            begun: AtomicUsize::new(0),
            finished: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl ReaperStore for Store {
    async fn begin(&self) -> Result<Option<Box<dyn ReapPass>>, PortError> {
        self.begun.fetch_add(1, Ordering::SeqCst);
        let result = match self.pass {
            Pass::Skipped => Ok(None),
            Pass::Fails => Err(PortError("database unreachable".into())),
            Pass::Takes(duration) => {
                tokio::time::sleep(duration).await;
                Ok(None)
            }
        };
        self.finished.fetch_add(1, Ordering::SeqCst);
        result
    }
}

struct NoMediaServer;

#[async_trait]
impl StreamProbe for NoMediaServer {
    async fn live_paths(&self) -> Result<Vec<StreamPath>, PortError> {
        unreachable!("no pass gets this far")
    }
}

#[async_trait]
impl StreamTerminator for NoMediaServer {
    async fn kick(&self, _: &StreamPath) -> Result<(), PortError> {
        unreachable!("no pass gets this far")
    }
}

fn spawn(store: Arc<Store>) -> reaper::ReaperHandle {
    reaper::spawn(Reaper {
        store,
        probe: Arc::new(NoMediaServer),
        ingest: Arc::new(MediaMtxIngestRouter),
        terminator: Arc::new(NoMediaServer),
    })
}

#[tokio::test(start_paused = true)]
async fn a_pass_runs_at_once_and_then_every_interval() {
    let store = Store::new(Pass::Skipped);
    let handle = spawn(store.clone());

    tokio::time::sleep(REAP_INTERVAL * 3 + Duration::from_millis(1)).await;

    assert_eq!(store.begun.load(Ordering::SeqCst), 4);
    handle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_failed_pass_does_not_stop_the_next() {
    let store = Store::new(Pass::Fails);
    let handle = spawn(store.clone());

    tokio::time::sleep(REAP_INTERVAL * 3 + Duration::from_millis(1)).await;

    assert_eq!(store.begun.load(Ordering::SeqCst), 4);
    handle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn stopping_waits_for_the_pass_in_progress() {
    let store = Store::new(Pass::Takes(Duration::from_secs(2)));
    let handle = spawn(store.clone());
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(store.begun.load(Ordering::SeqCst), 1);

    let stopping = Instant::now();
    handle.shutdown().await;

    assert_eq!(
        store.finished.load(Ordering::SeqCst),
        1,
        "the pass finished"
    );
    assert!(stopping.elapsed() >= Duration::from_millis(1999));
    tokio::time::sleep(REAP_INTERVAL * 2).await;
    assert_eq!(
        store.begun.load(Ordering::SeqCst),
        1,
        "no pass after stopping"
    );
}
