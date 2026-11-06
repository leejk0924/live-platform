//! Every error response leaves as `{"code","message","traceId"}`.
//!
//! Handlers return [`ApiError`], which only marks the response with an
//! [`ErrorPayload`]. The [`trace_id`] middleware owns the trace id, so it is
//! the one that writes the JSON body.

use std::{any::Any, fmt::Display};

use axum::{
    Extension,
    body::Body,
    extract::Request,
    http::{
        HeaderValue, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE},
    },
    middleware::Next,
    response::{IntoResponse, Response},
};
use domain::error::{DomainError, ErrorKind};
use serde::Serialize;
use tracing::Instrument;
use uuid::Uuid;

pub const TRACE_ID_HEADER: &str = "x-trace-id";

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    /// Logs `err` and answers with a generic 500, so no internal detail
    /// (SQL, paths, backtraces) reaches the client.
    pub fn internal(err: impl Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL",
            message: "internal server error".into(),
        }
    }

    pub fn method_not_allowed() -> Self {
        Self {
            status: StatusCode::METHOD_NOT_ALLOWED,
            code: "METHOD_NOT_ALLOWED",
            message: "method not allowed".into(),
        }
    }

    pub fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: "resource not found".into(),
        }
    }
}

impl<E: DomainError> From<E> for ApiError {
    fn from(err: E) -> Self {
        let status = match err.kind() {
            ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
            ErrorKind::Unauthenticated => StatusCode::UNAUTHORIZED,
            ErrorKind::Forbidden => StatusCode::FORBIDDEN,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::Conflict => StatusCode::CONFLICT,
        };
        Self {
            status,
            code: err.code(),
            message: err.to_string(),
        }
    }
}

/// Turns a handler panic into the generic 500. The payload goes to the log
/// only.
pub fn panic_response(payload: Box<dyn Any + Send + 'static>) -> Response {
    let detail = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload");
    ApiError::internal(format_args!("handler panicked: {detail}")).into_response()
}

#[derive(Debug, Clone)]
struct ErrorPayload {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let payload = ErrorPayload {
            code: self.code,
            message: self.message,
        };
        (self.status, Extension(payload)).into_response()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    trace_id: &'a str,
}

/// Gives each request a trace id, runs it inside a span carrying that id,
/// returns the id in `x-trace-id`, and renders [`ApiError`] responses.
pub async fn trace_id(request: Request, next: Next) -> Response {
    let trace_id = Uuid::new_v4().simple().to_string();
    let span = tracing::info_span!(
        "request",
        trace_id = %trace_id,
        method = %request.method(),
        path = %request.uri().path(),
    );
    let mut response = next.run(request).instrument(span).await;

    if let Some(payload) = response.extensions_mut().remove::<ErrorPayload>() {
        let body = ErrorBody {
            code: payload.code,
            message: &payload.message,
            trace_id: &trace_id,
        };
        let json = serde_json::to_vec(&body).expect("a struct of strings always serializes");
        let (mut parts, _) = response.into_parts();
        parts
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        parts.headers.remove(CONTENT_LENGTH);
        response = Response::from_parts(parts, Body::from(json));
    }

    let header = HeaderValue::from_str(&trace_id).expect("hex is a valid header value");
    response.headers_mut().insert(TRACE_ID_HEADER, header);
    response
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::to_bytes,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::test_support::LogBuffer;

    #[derive(Debug, thiserror::Error)]
    #[error("stream is already live")]
    struct AlreadyLive;

    impl DomainError for AlreadyLive {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Conflict
        }

        fn code(&self) -> &'static str {
            "STREAM_ALREADY_LIVE"
        }
    }

    fn router() -> Router {
        Router::new()
            .route("/conflict", get(|| async { ApiError::from(AlreadyLive) }))
            .route(
                "/boom",
                get(|| async { ApiError::internal("relation \"users\" does not exist") }),
            )
            .route("/ok", get(|| async { "fine" }))
            .layer(middleware::from_fn(trace_id))
    }

    async fn call(path: &str) -> (StatusCode, String, Option<String>, Vec<u8>) {
        let response = router()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let trace = response.headers()[TRACE_ID_HEADER]
            .to_str()
            .unwrap()
            .to_owned();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned());
        let body = to_bytes(response.into_body(), 4096).await.unwrap().to_vec();
        (status, trace, content_type, body)
    }

    #[tokio::test]
    async fn domain_error_becomes_the_envelope_with_the_header_trace_id() {
        let (status, trace, content_type, body) = call("/conflict").await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(content_type.as_deref(), Some("application/json"));
        assert_eq!(trace.len(), 32);
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body,
            json!({"code": "STREAM_ALREADY_LIVE", "message": "stream is already live", "traceId": trace})
        );
    }

    #[test]
    fn error_kinds_map_to_statuses() {
        #[derive(Debug, thiserror::Error)]
        #[error("x")]
        struct Kind(ErrorKind);
        impl DomainError for Kind {
            fn kind(&self) -> ErrorKind {
                self.0
            }
            fn code(&self) -> &'static str {
                "X"
            }
        }

        for (kind, status) in [
            (ErrorKind::InvalidInput, StatusCode::BAD_REQUEST),
            (ErrorKind::Unauthenticated, StatusCode::UNAUTHORIZED),
            (ErrorKind::Forbidden, StatusCode::FORBIDDEN),
            (ErrorKind::NotFound, StatusCode::NOT_FOUND),
            (ErrorKind::Conflict, StatusCode::CONFLICT),
        ] {
            assert_eq!(ApiError::from(Kind(kind)).status, status);
        }
    }

    #[tokio::test]
    async fn internal_error_hides_the_detail_and_logs_it() {
        let logs = LogBuffer::default();
        let _guard = tracing::subscriber::set_default(logs.subscriber());

        let (status, trace, _, body) = call("/boom").await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body,
            json!({"code": "INTERNAL", "message": "internal server error", "traceId": trace})
        );
        let logged = logs.contents();
        assert!(
            logged.contains("relation \\\"users\\\" does not exist"),
            "{logged}"
        );
    }

    #[tokio::test]
    async fn success_keeps_its_body_and_gets_a_trace_id() {
        let (status, trace, _, body) = call("/ok").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(trace.len(), 32);
        assert_eq!(body, b"fine");
    }
}
