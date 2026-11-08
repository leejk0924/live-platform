//! `/internal/media/*`: what MediaMTX calls. Caddy proxies only `/api/*`, so
//! these are reachable on the compose network only, and every request must
//! also carry the shared secret as HTTP Basic credentials. MediaMTX cannot
//! sign a request, so the secret is all it can present.

use std::sync::Arc;

use application::{
    media::{Admitted, Denied, MediaAccess, Observed, PublisherSignal},
    ports::media::StreamPath,
};
use axum::{
    Router,
    body::Bytes,
    extract::{Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use base64ct::{Base64, Encoding};
use domain::secret::Secret;
use serde::Deserialize;
use subtle::ConstantTimeEq;

use crate::{AppState, error::ApiError};

pub fn routes(secret: Arc<Secret<String>>) -> Router<AppState> {
    Router::new()
        .route("/media/auth", post(auth))
        .route("/media/hooks", post(hook))
        .layer(middleware::from_fn_with_state(secret, require_secret))
}

/// The password of `Authorization: Basic`, compared in constant time. The
/// user name is ignored.
async fn require_secret(
    State(secret): State<Arc<Secret<String>>>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Basic "))
        .and_then(|encoded| Base64::decode_vec(encoded.trim()).ok());
    let matches = presented.is_some_and(|decoded| {
        decoded
            .iter()
            .position(|&b| b == b':')
            .is_some_and(|colon| {
                decoded[colon + 1..]
                    .ct_eq(secret.expose().as_bytes())
                    .into()
            })
    });
    if !matches {
        return ApiError::unauthenticated().into_response();
    }
    next.run(request).await
}

/// The JSON MediaMTX posts for each authentication (`authMethod: http`).
/// MediaMTX also sends `user`, `password`, `ip`, `protocol`, `id`, `query`
/// and `userAgent`; none of them decide anything here.
#[derive(Deserialize)]
struct AuthRequest {
    action: String,
    #[serde(default)]
    path: String,
    /// The bearer token of the request MediaMTX is deciding on.
    #[serde(default)]
    token: String,
}

/// 2xx lets the request in; anything else keeps it out. A refusal tells
/// MediaMTX nothing more than that.
async fn auth(State(state): State<AppState>, body: Bytes) -> Result<StatusCode, ApiError> {
    let request: AuthRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::invalid_request())?;
    let token = (!request.token.is_empty()).then(|| Secret::new(request.token));
    let access = match request.action.as_str() {
        "publish" => MediaAccess::Publish {
            path: StreamPath::new(request.path.clone()),
            token,
        },
        "api" => MediaAccess::Control { token },
        _ => MediaAccess::Other,
    };
    let decision = state
        .broadcasts
        .admit
        .run(access)
        .await
        .map_err(ApiError::internal)?;
    let action = request.action.as_str();
    let path = request.path.as_str();
    match decision {
        Ok(Admitted { ticket }) => {
            match ticket {
                Some((jti, _)) => {
                    tracing::info!(action, path, jti = jti.as_str(), "media access admitted");
                }
                // The reaper's own Control API calls, every few seconds.
                None => tracing::debug!(action, "media access admitted"),
            }
            Ok(StatusCode::NO_CONTENT)
        }
        Err(Denied { reason, ticket }) => {
            let jti = ticket.as_ref().map(|jti| jti.as_str());
            tracing::info!(action, path, jti, ?reason, "media access denied");
            Err(ApiError::unauthenticated())
        }
    }
}

/// What MediaMTX's `runOnAvailable` / `runOnUnavailable` commands post.
#[derive(Deserialize)]
struct HookRequest {
    event: HookEvent,
    path: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HookEvent {
    Available,
    Unavailable,
}

/// Always 204 once the request is understood: a report that changes nothing
/// is normal, not an error for MediaMTX to retry or log.
async fn hook(State(state): State<AppState>, body: Bytes) -> Result<StatusCode, ApiError> {
    let request: HookRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::invalid_request())?;
    let signal = match request.event {
        HookEvent::Available => PublisherSignal::Arrived,
        HookEvent::Unavailable => PublisherSignal::Left,
    };
    let observed = state
        .broadcasts
        .track
        .run(&StreamPath::new(request.path.clone()), signal)
        .await
        .map_err(ApiError::internal)?;
    let (event, path) = (request.event, request.path.as_str());
    match observed {
        Observed::UnknownPath => tracing::warn!(?event, path, "hook for an unknown path"),
        Observed::Changed(id) => {
            tracing::info!(?event, path, broadcast = %id, "status changed by hook")
        }
        Observed::Unchanged(id) => {
            tracing::info!(?event, path, broadcast = %id, "hook changed nothing")
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
