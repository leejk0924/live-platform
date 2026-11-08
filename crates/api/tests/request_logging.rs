//! Its own test binary: tracing caches per-callsite interest process-wide, so a
//! concurrent test hitting the request span without a subscriber could hide
//! the span from the scoped subscriber below.

use std::{
    io,
    sync::{Arc, Mutex},
};

use api::error::{ApiError, TRACE_ID_HEADER, trace_id};
use axum::{Router, body::Body, http::Request, middleware, routing::get};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

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

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn logs_inside_a_request_carry_its_trace_id() {
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_current_span(true)
        .with_writer(logs.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let router = Router::new()
        .route("/boom", get(|| async { ApiError::internal("disk full") }))
        .layer(middleware::from_fn(trace_id));
    let response = router
        .oneshot(Request::get("/boom").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let trace = response.headers()[TRACE_ID_HEADER].to_str().unwrap();
    let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("disk full"), "{logged}");
    assert!(
        logged.contains(&format!(r#""trace_id":"{trace}""#)),
        "log should carry the trace id {trace}: {logged}"
    );
}
