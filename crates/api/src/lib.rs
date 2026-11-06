//! HTTP inbound adapter and composition root. A library so the binary and the
//! tests share it, and items the next handlers will use are not dead code.

pub mod auth;
pub mod broadcast;
pub mod config;
pub mod csrf;
pub mod error;
pub mod json;
pub mod rate_limit;
#[cfg(test)]
mod test_support;

use std::sync::Arc;

use application::ports::id::IdGenerator;
use axum::{Router, middleware, routing::get};
use tower_http::catch_panic::CatchPanicLayer;

use crate::{
    auth::{AuthPorts, AuthUseCases},
    broadcast::{BroadcastPorts, BroadcastUseCases},
    config::Origin,
    error::ApiError,
    rate_limit::RateLimits,
};

/// Dependencies the handlers share, assembled in `main`.
#[derive(Clone)]
pub struct AppState {
    pub ids: Arc<dyn IdGenerator>,
    pub auth: Arc<AuthUseCases>,
    pub broadcasts: Arc<BroadcastUseCases>,
    pub allowed_origins: Arc<[Origin]>,
    pub limits: Arc<RateLimits>,
}

impl AppState {
    pub fn new(
        ports: AuthPorts,
        broadcast_ports: BroadcastPorts,
        allowed_origins: Vec<Origin>,
        demo_login_enabled: bool,
    ) -> Self {
        let broadcasts =
            BroadcastUseCases::new(broadcast_ports, ports.ids.clone(), ports.users.clone());
        Self {
            ids: ports.ids.clone(),
            auth: Arc::new(AuthUseCases::new(ports, demo_login_enabled)),
            broadcasts: Arc::new(broadcasts),
            allowed_origins: allowed_origins.into(),
            limits: Arc::new(RateLimits::default()),
        }
    }
}

pub fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/healthz", get(healthz))
        .nest("/auth", auth::routes())
        .merge(broadcast::routes())
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state.allowed_origins),
            csrf::guard,
        ));
    with_envelope(Router::new().nest("/api", api)).with_state(state)
}

/// Wraps `routes` so that every error, including the ones axum produces
/// itself, leaves as the JSON envelope with a trace id.
fn with_envelope(routes: Router<AppState>) -> Router<AppState> {
    routes
        .fallback(|| async { ApiError::not_found() })
        .method_not_allowed_fallback(|| async { ApiError::method_not_allowed() })
        // Inside trace_id, so the panic response still gets the envelope.
        .layer(CatchPanicLayer::custom(error::panic_response))
        .layer(middleware::from_fn(error::trace_id))
}

async fn healthz() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::test_support::test_state;

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
        let router =
            with_envelope(Router::new().route("/api/panic", get(panics))).with_state(test_state());
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
}
