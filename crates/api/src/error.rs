//! Every error response leaves as `{"code","message","traceId"}`.
//!
//! Handlers return [`ApiError`], which only marks the response with an
//! [`ErrorPayload`]. The [`trace_id`] middleware owns the trace id, so it is
//! the one that writes the JSON body.

use std::{any::Any, fmt::Display, time::Duration};

use axum::{
    Extension,
    body::Body,
    extract::Request,
    http::{
        HeaderValue, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE, RETRY_AFTER},
    },
    middleware::Next,
    response::{IntoResponse, Response},
};
use domain::error::{DomainError, ErrorKind};
use serde::Serialize;
use tracing::Instrument;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    audit::{AuditEvent, audit},
    rate_limit::ClientKey,
};

pub const TRACE_ID_HEADER: &str = "x-trace-id";

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    retry_after: Option<Duration>,
}

impl ApiError {
    /// Logs `err` and answers with a generic 500, so no internal detail
    /// (SQL, paths, backtraces) reaches the client.
    pub fn internal(err: impl Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            "internal server error",
        )
    }

    pub fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "METHOD_NOT_ALLOWED",
            "method not allowed",
        )
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", "resource not found")
    }

    /// The request body was missing, not JSON, or not the expected shape.
    pub fn invalid_request() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
            "request body is invalid",
        )
    }

    /// A body extractor refused the body: 413 when it was over the limit,
    /// [`ApiError::invalid_request`] otherwise.
    pub fn rejected_body(status: StatusCode) -> Self {
        if status == StatusCode::PAYLOAD_TOO_LARGE {
            Self::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PAYLOAD_TOO_LARGE",
                "request body is too large",
            )
        } else {
            Self::invalid_request()
        }
    }

    /// 503 rather than 408, which browsers may retry on their own.
    pub fn timed_out() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "REQUEST_TIMEOUT",
            "request took too long",
        )
    }

    pub fn invalid_query() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_QUERY",
            "query parameters are invalid",
        )
    }

    pub fn if_match_required() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "IF_MATCH_REQUIRED",
            "If-Match header with the version to update is required",
        )
    }

    pub fn invalid_if_match() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_IF_MATCH",
            "If-Match must be one version tag such as \"3\"",
        )
    }

    /// A caller without valid credentials, where no more detail is given.
    pub fn unauthenticated() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHENTICATED",
            "authentication required",
        )
    }

    /// A signed link whose signature does not match or whose time ran out.
    pub fn invalid_signature() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "INVALID_SIGNATURE",
            "the link is invalid or has expired",
        )
    }

    pub fn csrf_rejected() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "CSRF_REJECTED",
            "request origin is not allowed",
        )
    }

    /// Donations need Toss keys, and none are configured.
    pub fn payments_disabled() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PAYMENTS_DISABLED",
            "donations are not available on this server",
        )
    }

    /// The payment provider gave no answer to rely on; the same request may
    /// succeed later.
    pub fn payment_gateway_unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PAYMENT_GATEWAY_UNAVAILABLE",
            "the payment service is not responding, try again later",
        )
    }

    /// Like [`ApiError::internal`], but 503 for a caller that retries on it,
    /// such as a webhook sender.
    pub fn internal_retry_later(err: impl Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "INTERNAL",
            "internal server error, try again later",
        )
    }

    /// Answers 429 with `Retry-After` in whole seconds, rounded up.
    pub fn rate_limited(retry_after: Duration) -> Self {
        Self {
            retry_after: Some(retry_after),
            ..Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "RATE_LIMITED",
                "too many requests, try again later",
            )
        }
    }

    /// Answers 429 without `Retry-After`: a slot frees only when one of the
    /// client's open streams closes.
    pub fn too_many_streams() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "TOO_MANY_STREAMS",
            "too many event streams open from this client",
        )
    }

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    fn new(status: StatusCode, code: &'static str, message: &str) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after: None,
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
            retry_after: None,
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
        let mut response = (self.status, Extension(payload)).into_response();
        if let Some(wait) = self.retry_after {
            let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(secs.max(1)));
        }
        response
    }
}

/// The body of every error response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody<'a> {
    /// Stable SCREAMING_SNAKE identifier to match on, such as `VERSION_CONFLICT`.
    code: &'a str,
    /// Human-readable; do not match on it.
    message: &'a str,
    /// Also in the `x-trace-id` header; quote it when reporting a problem.
    trace_id: &'a str,
}

/// Gives each request a trace id, runs it inside a span carrying that id and
/// the client (the signed-in user is recorded on it once known), returns the
/// id in `x-trace-id`, renders [`ApiError`] responses, and audits every 403.
pub async fn trace_id(request: Request, next: Next) -> Response {
    let trace_id = Uuid::new_v4().simple().to_string();
    let client = ClientKey::of(request.headers(), request.extensions());
    let span = tracing::info_span!(
        "request",
        trace_id = %trace_id,
        client = client.as_str(),
        user = tracing::field::Empty,
        method = %request.method(),
        path = %request.uri().path(),
    );
    let mut response = next.run(request).instrument(span.clone()).await;
    let payload = response.extensions_mut().remove::<ErrorPayload>();

    if response.status() == StatusCode::FORBIDDEN {
        let code = payload.as_ref().map(|payload| payload.code);
        span.in_scope(|| audit!(warn, AuditEvent::Forbidden, code, "request forbidden"));
    }

    if let Some(payload) = payload {
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
