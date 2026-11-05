//! HTTP inbound adapter and composition root. A library so the binary and the
//! tests share it, and items the next handlers will use are not dead code.

pub mod config;
pub mod error;
pub mod json;
#[cfg(test)]
mod test_support;

use axum::{Router, middleware, routing::get};

use crate::error::ApiError;

pub fn app() -> Router {
    Router::new()
        .nest("/api", Router::new().route("/healthz", get(healthz)))
        .fallback(|| async { ApiError::not_found() })
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

    #[tokio::test]
    async fn healthz_returns_ok() {
        let response = app()
            .oneshot(Request::get("/api/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64).await.unwrap();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn unknown_route_is_a_not_found_envelope() {
        for path in ["/api/nope", "/nope"] {
            let response = app()
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
