//! HTTP inbound adapter and composition root. A library so the binary and the
//! tests share it, and items the next handlers will use are not dead code.

pub mod auth;
pub mod broadcast;
pub mod config;
pub mod csrf;
pub mod error;
pub mod internal;
pub mod json;
pub mod live;
pub mod openapi;
pub mod rate_limit;
pub mod reaper;
#[cfg(test)]
mod test_support;

use std::{sync::Arc, time::Duration};

use application::ports::id::IdGenerator;
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use domain::secret::Secret;
use infrastructure::media::PublicJwk;
use tower_http::catch_panic::CatchPanicLayer;

use crate::{
    auth::{AuthPorts, AuthUseCases},
    broadcast::{BroadcastPorts, BroadcastUseCases},
    config::Origin,
    error::ApiError,
    rate_limit::RateLimits,
};

/// How long a request may take to produce its response head. Routes that
/// hold a connection open, such as event streams, are exempt.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest request body a body extractor reads. Every body the API takes
/// is a small JSON document.
pub const BODY_LIMIT: usize = 64 * 1024;

/// Dependencies the handlers share, assembled in `main`.
#[derive(Clone)]
pub struct AppState {
    pub ids: Arc<dyn IdGenerator>,
    pub auth: Arc<AuthUseCases>,
    pub broadcasts: Arc<BroadcastUseCases>,
    pub allowed_origins: Arc<[Origin]>,
    pub limits: Arc<RateLimits>,
    /// Served as the JWKS that verifies publish tokens.
    pub publish_jwk: Arc<PublicJwk>,
    /// What MediaMTX must present on `/internal/*`.
    pub media_secret: Arc<Secret<String>>,
}

impl AppState {
    pub fn new(
        ports: AuthPorts,
        broadcast_ports: BroadcastPorts,
        allowed_origins: Vec<Origin>,
        demo_login_enabled: bool,
        publish_jwk: PublicJwk,
        media_secret: Secret<String>,
    ) -> Self {
        let broadcasts =
            BroadcastUseCases::new(broadcast_ports, ports.ids.clone(), ports.users.clone());
        Self {
            ids: ports.ids.clone(),
            auth: Arc::new(AuthUseCases::new(ports, demo_login_enabled)),
            broadcasts: Arc::new(broadcasts),
            allowed_origins: allowed_origins.into(),
            limits: Arc::new(RateLimits::default()),
            publish_jwk: Arc::new(publish_jwk),
            media_secret: Arc::new(media_secret),
        }
    }
}

pub fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/healthz", get(healthz))
        .route("/openapi.json", get(openapi::spec))
        .nest("/auth", auth::routes())
        .merge(broadcast::routes())
        .merge(live::routes())
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state.allowed_origins),
            csrf::guard,
        ));
    let internal = internal::routes(Arc::clone(&state.media_secret));
    let bounded = Router::new().nest("/api", api).nest("/internal", internal);
    // Routes that hold a connection open, such as SSE (#11), go in the
    // second router: `bounded` cuts them off at `REQUEST_TIMEOUT`.
    with_envelope(bounded, Router::new()).with_state(state)
}

/// Joins the routes under [`REQUEST_TIMEOUT`] with the `long_lived` ones
/// exempt from it, limits bodies to [`BODY_LIMIT`], and makes every error,
/// including the ones axum produces itself, leave as the JSON envelope with
/// a trace id.
fn with_envelope(bounded: Router<AppState>, long_lived: Router<AppState>) -> Router<AppState> {
    bounded
        .layer(middleware::from_fn(time_limit))
        .merge(long_lived)
        .fallback(|| async { ApiError::not_found() })
        .method_not_allowed_fallback(|| async { ApiError::method_not_allowed() })
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        // Inside trace_id, so the panic response still gets the envelope.
        .layer(CatchPanicLayer::custom(error::panic_response))
        .layer(middleware::from_fn(error::trace_id))
}

/// Drops the handler when it runs past [`REQUEST_TIMEOUT`]. A database
/// transaction it held rolls back; a kick it had not sent yet is sent by the
/// reaper's next pass.
async fn time_limit(request: Request, next: Next) -> Response {
    match tokio::time::timeout(REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => {
            tracing::warn!("request dropped after the time limit");
            ApiError::timed_out().into_response()
        }
    }
}

async fn healthz() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use base64ct::{Base64, Encoding};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        csrf::{REQUESTED_WITH_HEADER, REQUESTED_WITH_VALUE},
        test_support::{TEST_MEDIA_SECRET, TEST_ORIGIN, test_state},
    };

    #[tokio::test]
    async fn healthz_returns_ok() {
        let response = app(test_state())
            .oneshot(Request::get("/api/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64).await.unwrap();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn openapi_json_is_served() {
        let response = app(test_state())
            .oneshot(
                Request::get("/api/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap())
                .unwrap();
        assert_eq!(body["info"]["title"], "live-platform API");
        assert!(body["paths"]["/api/broadcasts/{id}"]["put"].is_object());
    }

    async fn envelope_of(router: Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let trace = response.headers()[error::TRACE_ID_HEADER]
            .to_str()
            .unwrap()
            .to_owned();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                .unwrap_or_else(|err| panic!("{status} body is not JSON: {err}"));
        assert_eq!(body["traceId"], trace, "{body}");
        (status, body)
    }

    #[tokio::test]
    async fn wrong_method_is_a_method_not_allowed_envelope() {
        let request = Request::post("/api/healthz").body(Body::empty()).unwrap();
        let (status, body) = envelope_of(app(test_state()), request).await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["code"], "METHOD_NOT_ALLOWED");
    }

    #[tokio::test]
    async fn handler_panic_is_an_internal_envelope_without_the_payload() {
        async fn panics() -> &'static str {
            panic!("secret detail")
        }
        let router = with_envelope(
            Router::new().route("/api/panic", get(panics)),
            Router::new(),
        )
        .with_state(test_state());
        let request = Request::get("/api/panic").body(Body::empty()).unwrap();
        let (status, body) = envelope_of(router, request).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "INTERNAL");
        assert!(!body.to_string().contains("secret detail"), "{body}");
    }

    #[tokio::test]
    async fn unknown_route_is_a_not_found_envelope() {
        for path in ["/api/nope", "/nope"] {
            let response = app(test_state())
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            let trace = response.headers()[error::TRACE_ID_HEADER]
                .to_str()
                .unwrap()
                .to_owned();
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                    .unwrap();
            assert_eq!(
                body,
                json!({"code": "NOT_FOUND", "message": "resource not found", "traceId": trace})
            );
        }
    }

    fn login_with_body_of(len: usize) -> Request<Body> {
        Request::post("/api/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, TEST_ORIGIN)
            .header(REQUESTED_WITH_HEADER, REQUESTED_WITH_VALUE)
            .body(Body::from(vec![b' '; len]))
            .unwrap()
    }

    fn media_auth_with_body_of(len: usize) -> Request<Body> {
        let basic = Base64::encode_string(format!("mediamtx:{TEST_MEDIA_SECRET}").as_bytes());
        Request::post("/internal/media/auth")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Basic {basic}"))
            .body(Body::from(vec![b' '; len]))
            .unwrap()
    }

    #[tokio::test]
    async fn body_over_the_limit_is_a_payload_too_large_envelope() {
        for request in [
            login_with_body_of(BODY_LIMIT + 1),
            media_auth_with_body_of(BODY_LIMIT + 1),
        ] {
            let path = request.uri().path().to_owned();
            let response = app(test_state()).oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE, "{path}");
            let trace = response.headers()[error::TRACE_ID_HEADER]
                .to_str()
                .unwrap()
                .to_owned();
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                    .unwrap();
            assert_eq!(
                body,
                json!({
                    "code": "PAYLOAD_TOO_LARGE",
                    "message": "request body is too large",
                    "traceId": trace,
                }),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn body_at_the_limit_is_read() {
        for request in [
            login_with_body_of(BODY_LIMIT),
            media_auth_with_body_of(BODY_LIMIT),
        ] {
            let path = request.uri().path().to_owned();
            let (status, body) = envelope_of(app(test_state()), request).await;

            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(body["code"], "INVALID_REQUEST", "{path}");
        }
    }

    async fn outlives_the_time_limit() -> &'static str {
        tokio::time::sleep(REQUEST_TIMEOUT + Duration::from_secs(1)).await;
        "done"
    }

    #[tokio::test(start_paused = true)]
    async fn bounded_request_past_the_time_limit_is_a_timeout_envelope() {
        let router = with_envelope(
            Router::new().route("/api/slow", get(outlives_the_time_limit)),
            Router::new(),
        )
        .with_state(test_state());
        let request = Request::get("/api/slow").body(Body::empty()).unwrap();
        let (status, body) = envelope_of(router, request).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "REQUEST_TIMEOUT");
    }

    #[tokio::test(start_paused = true)]
    async fn long_lived_route_is_exempt_from_the_time_limit() {
        let router = with_envelope(
            Router::new(),
            Router::new().route("/api/slow", get(outlives_the_time_limit)),
        )
        .with_state(test_state());
        let response = router
            .oneshot(Request::get("/api/slow").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64).await.unwrap();
        assert_eq!(&body[..], b"done");
    }
}
