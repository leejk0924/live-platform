//! Its own test binary: tracing caches per-callsite interest process-wide, so a
//! concurrent test hitting the request span without a subscriber could hide
//! the span from the scoped subscriber below.

use std::{
    io,
    sync::{Arc, Mutex},
};

use api::{
    config::Origin,
    csrf,
    error::{ApiError, TRACE_ID_HEADER, trace_id},
};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
};
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

impl LogBuffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn capture() -> (LogBuffer, tracing::subscriber::DefaultGuard) {
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_current_span(true)
        .with_writer(logs.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (logs, guard)
}

#[tokio::test]
async fn logs_inside_a_request_carry_its_trace_id() {
    let (logs, _guard) = capture();

    let router = Router::new()
        .route("/boom", get(|| async { ApiError::internal("disk full") }))
        .layer(middleware::from_fn(trace_id));
    let response = router
        .oneshot(Request::get("/boom").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let trace = response.headers()[TRACE_ID_HEADER].to_str().unwrap();
    let logged = logs.contents();
    assert!(logged.contains("disk full"), "{logged}");
    assert!(
        logged.contains(&format!(r#""trace_id":"{trace}""#)),
        "log should carry the trace id {trace}: {logged}"
    );
}

#[tokio::test]
async fn a_forbidden_response_is_one_audit_line_with_the_request_context() {
    let (logs, _guard) = capture();
    let allowed: Arc<[Origin]> = vec![Origin::parse("https://app.test").unwrap()].into();
    let router = Router::new()
        .route("/change", post(|| async { "changed" }))
        .layer(middleware::from_fn_with_state(allowed, csrf::guard))
        .layer(middleware::from_fn(trace_id));

    let response = router
        .oneshot(
            Request::post("/change")
                .header("x-forwarded-for", "203.0.113.9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let trace = response.headers()[TRACE_ID_HEADER].to_str().unwrap();
    let logged = logs.contents();
    let audits: Vec<&str> = logged
        .lines()
        .filter(|line| line.contains(r#""target":"audit""#))
        .collect();
    assert_eq!(audits.len(), 1, "{logged}");
    let line = audits[0];
    for expected in [
        r#""level":"WARN""#.to_owned(),
        r#""event":"forbidden""#.to_owned(),
        r#""code":"CSRF_REJECTED""#.to_owned(),
        format!(r#""trace_id":"{trace}""#),
        r#""client":"203.0.113.9""#.to_owned(),
        r#""path":"/change""#.to_owned(),
    ] {
        assert!(line.contains(&expected), "{expected} missing from {line}");
    }
}
