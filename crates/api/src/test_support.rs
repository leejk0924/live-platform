use std::{
    io,
    sync::{Arc, Mutex},
};

use application::ports::id::{IdGenerationError, IdGenerator};
use domain::id::SnowflakeId;
use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;

use crate::AppState;

struct UnusedIds;

impl IdGenerator for UnusedIds {
    fn next_id(&self) -> Result<SnowflakeId, IdGenerationError> {
        Err(IdGenerationError::LeaseExpired)
    }
}

pub fn test_state() -> AppState {
    AppState {
        ids: Arc::new(UnusedIds),
    }
}

/// Captures JSON log lines so tests can assert what reached the logs.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub fn subscriber(&self) -> impl Subscriber + Send + Sync + 'static {
        tracing_subscriber::fmt()
            .json()
            .with_writer(self.clone())
            .finish()
    }

    pub fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
