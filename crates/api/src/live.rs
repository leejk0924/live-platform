//! Going live, ending and watching a broadcast, and the public key that
//! verifies publish tokens.

use application::{
    live::PublishGrant,
    ports::media::{PlaybackKind, PlaybackSource, PublishCredential},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use infrastructure::media::PublicJwk;
use serde::Serialize;
use tracing::Instrument;
use utoipa::ToSchema;

use crate::{
    AppState,
    audit::{AuditEvent, audit},
    auth::AuthUser,
    broadcast::{BroadcastDto, api_error, broadcast_id, tagged},
    error::{ApiError, ErrorBody},
};

/// The same for every viewer. As with the list, a few seconds of staleness
/// only delays a viewer noticing that the broadcast went live or ended.
pub const PLAYBACK_CACHE_CONTROL: &str = "public, max-age=5";

/// The key changes only when the operator rotates it, so a short shared cache
/// costs nothing.
pub const JWKS_CACHE_CONTROL: &str = "public, max-age=300";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/broadcasts/{id}/start", post(start))
        .route("/broadcasts/{id}/stop", post(stop))
        .route("/broadcasts/{id}/playback", get(playback))
        .route("/.well-known/jwks.json", get(jwks))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = PublishGrant)]
pub struct PublishGrantDto {
    /// Same-origin WHIP endpoint to POST the SDP offer to.
    #[schema(example = "/media/webrtc/live/112007303372210176/whip")]
    whip_url: String,
    /// Send as `Authorization: Bearer <token>` on the WHIP request. Allows
    /// publishing to this broadcast only, once.
    token: String,
    /// Seconds the token may wait before its one use. The publish session it
    /// opens lasts until the publisher leaves or the broadcast ends.
    expires_in: u64,
}

impl From<PublishGrant> for PublishGrantDto {
    fn from(grant: PublishGrant) -> Self {
        let PublishCredential::Bearer { token, expires_in } = grant.credential;
        Self {
            whip_url: grant.whip_url,
            token: token.expose().clone(),
            expires_in: expires_in.as_secs(),
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/broadcasts/{id}/start",
    operation_id = "startBroadcast",
    tag = "broadcasts",
    summary = "Go live, or get a fresh publish token for a live or interrupted broadcast; owner only",
    params(("id" = String, Path, description = "Broadcast id")),
    security(("session" = [])),
    responses(
        (status = 200, body = PublishGrantDto,
            description = "`READY` became `LIVE`, or the broadcast was already `LIVE` or `INTERRUPTED` and only a new token was issued.",
            headers(("Cache-Control" = String, description = "`no-store`"))),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`NOT_BROADCAST_OWNER` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
        (status = 409, body = ErrorBody, description = "`BROADCAST_NOT_STARTABLE`: ended; `OWNER_ALREADY_LIVE`: the owner's other broadcast is live or interrupted"),
    ),
)]
pub(crate) async fn start(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let grant = state
        .broadcasts
        .start
        .run(actor, broadcast_id(&id)?)
        .await
        .map_err(api_error)?;
    Ok((
        [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(PublishGrantDto::from(grant)),
    )
        .into_response())
}

#[utoipa::path(
    post,
    path = "/api/broadcasts/{id}/stop",
    operation_id = "stopBroadcast",
    tag = "broadcasts",
    summary = "End a live or interrupted broadcast and disconnect its publisher; owner only",
    params(("id" = String, Path, description = "Broadcast id")),
    security(("session" = [])),
    responses(
        (status = 200, body = BroadcastDto, description = "Now `ENDED`.",
            headers(("ETag" = String, description = "The new version"))),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`NOT_BROADCAST_OWNER` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
        (status = 409, body = ErrorBody, description = "`BROADCAST_NOT_LIVE`: ready or already ended"),
    ),
)]
pub(crate) async fn stop(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let broadcast = broadcast_id(&id)?;
    // A client that hangs up drops this handler. Running the stop in its own
    // task keeps the kick from being cancelled after the commit.
    let stopping = tokio::spawn(
        async move {
            let stopped = state.broadcasts.stop.run(actor, broadcast).await?;
            match &stopped.kick_failure {
                None => audit!(
                    info,
                    AuditEvent::PublisherKicked,
                    %broadcast,
                    by = "owner",
                    "ended, publisher disconnected"
                ),
                Some(err) => audit!(
                    warn,
                    AuditEvent::KickFailed,
                    error = %err,
                    %broadcast,
                    by = "owner",
                    "ended, but the publisher was not disconnected"
                ),
            }
            Ok(stopped)
        }
        .in_current_span(),
    );
    let stopped = stopping
        .await
        .map_err(ApiError::internal)?
        .map_err(api_error)?;
    Ok(tagged(StatusCode::OK, stopped.broadcast))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
#[schema(as = PlaybackKind)]
pub enum PlaybackKindDto {
    /// Low-latency HLS playlist.
    LlHls,
    /// WHEP endpoint to POST a WebRTC SDP offer to.
    Whep,
}

impl From<PlaybackKind> for PlaybackKindDto {
    fn from(kind: PlaybackKind) -> Self {
        match kind {
            PlaybackKind::LlHls => Self::LlHls,
            PlaybackKind::Whep => Self::Whep,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[schema(as = PlaybackSource)]
pub struct PlaybackSourceDto {
    kind: PlaybackKindDto,
    /// Same-origin URL.
    #[schema(example = "/media/hls/live/112007303372210176/index.m3u8")]
    url: String,
}

impl From<PlaybackSource> for PlaybackSourceDto {
    fn from(source: PlaybackSource) -> Self {
        Self {
            kind: source.kind.into(),
            url: source.url,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/broadcasts/{id}/playback",
    operation_id = "getPlayback",
    tag = "broadcasts",
    summary = "Where to watch a live or interrupted broadcast",
    params(("id" = String, Path, description = "Broadcast id")),
    responses(
        (status = 200, body = Vec<PlaybackSourceDto>,
            description = "Sources in order of preference; the first is the default. An `INTERRUPTED` broadcast keeps its sources while its publisher may come back.",
            headers(("Cache-Control" = String, description = "`public, max-age=5`"))),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`, also for deleted broadcasts"),
        (status = 409, body = ErrorBody, description = "`BROADCAST_NOT_LIVE`: ready or ended"),
    ),
)]
pub(crate) async fn playback(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let sources = state
        .broadcasts
        .playback
        .run(broadcast_id(&id)?)
        .await
        .map_err(api_error)?;
    let body: Vec<PlaybackSourceDto> = sources.into_iter().map(Into::into).collect();
    Ok((
        [(
            CACHE_CONTROL,
            HeaderValue::from_static(PLAYBACK_CACHE_CONTROL),
        )],
        Json(body),
    )
        .into_response())
}

#[derive(Serialize, ToSchema)]
pub struct Jwks {
    keys: Vec<Jwk>,
}

/// An Ed25519 public key (RFC 8037).
#[derive(Serialize, ToSchema)]
pub struct Jwk {
    #[schema(example = "OKP")]
    kty: &'static str,
    #[schema(example = "Ed25519")]
    crv: &'static str,
    /// The public key, base64url.
    x: String,
    /// RFC 7638 thumbprint; matches the `kid` header of every token.
    kid: String,
    #[schema(example = "EdDSA")]
    alg: &'static str,
    #[serde(rename = "use")]
    #[schema(example = "sig")]
    use_: &'static str,
}

impl From<PublicJwk> for Jwks {
    fn from(jwk: PublicJwk) -> Self {
        Self {
            keys: vec![Jwk {
                kty: "OKP",
                crv: "Ed25519",
                x: jwk.x,
                kid: jwk.kid,
                alg: "EdDSA",
                use_: "sig",
            }],
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/.well-known/jwks.json",
    operation_id = "getJwks",
    tag = "broadcasts",
    summary = "Public key that verifies publish tokens",
    responses(
        (status = 200, body = Jwks, description = "The current signing key.",
            headers(("Cache-Control" = String, description = "`public, max-age=300`"))),
    ),
)]
pub(crate) async fn jwks(State(state): State<AppState>) -> Response {
    (
        [(CACHE_CONTROL, HeaderValue::from_static(JWKS_CACHE_CONTROL))],
        Json(Jwks::from(state.publish_jwk.as_ref().clone())),
    )
        .into_response()
}
