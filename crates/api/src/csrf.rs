//! CSRF guard for every state-changing request. `SameSite=Lax` alone is not
//! trusted: the request must name an allowed `Origin` and carry a header a
//! cross-site form cannot set.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{config::Origin, error::ApiError};

pub const REQUESTED_WITH_HEADER: &str = "x-requested-with";
pub const REQUESTED_WITH_VALUE: &str = "live-platform";

pub async fn guard(State(allowed): State<Arc<[Origin]>>, request: Request, next: Next) -> Response {
    let changes_state = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if changes_state && !is_same_app(&request, &allowed) {
        return ApiError::csrf_rejected().into_response();
    }
    next.run(request).await
}

fn is_same_app(request: &Request, allowed: &[Origin]) -> bool {
    let headers = request.headers();
    let origin_ok = headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| allowed.iter().any(|a| a.as_str() == origin));
    let marker_ok = headers
        .get(REQUESTED_WITH_HEADER)
        .is_some_and(|value| value == REQUESTED_WITH_VALUE);
    origin_ok && marker_ok
}
