//! `/api/auth/*`: translates HTTP to the auth use cases and owns the session
//! cookie.

use std::sync::Arc;

use application::{
    auth::{Authenticate, DemoLogIn, Error, LogIn, LogOut, SignUp, SignedIn},
    ports::{
        auth::{BreachedPasswords, PasswordHasher, SessionStore, UserRepository},
        id::IdGenerator,
    },
};
use axum::{
    Json, Router,
    extract::{FromRequestParts, State, rejection::JsonRejection},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{COOKIE, SET_COOKIE},
        request::Parts,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use domain::{
    auth::{AuthError, DemoAccount, SessionToken, User, UserId},
    id::SnowflakeId,
    secret::Secret,
};
use infrastructure::persistence::ABSOLUTE_TIMEOUT;
use serde::{Deserialize, Serialize};

use crate::{AppState, error::ApiError, rate_limit::ClientKey};

pub const SESSION_COOKIE: &str = "__Host-session";

pub struct AuthUseCases {
    pub sign_up: SignUp,
    pub log_in: LogIn,
    pub demo_log_in: DemoLogIn,
    pub log_out: LogOut,
    pub authenticate: Authenticate,
}

/// The ports the auth use cases are built from.
pub struct AuthPorts {
    pub ids: Arc<dyn IdGenerator>,
    pub users: Arc<dyn UserRepository>,
    pub sessions: Arc<dyn SessionStore>,
    pub hasher: Arc<dyn PasswordHasher>,
    pub breached: Arc<dyn BreachedPasswords>,
}

impl AuthUseCases {
    pub fn new(ports: AuthPorts, demo_login_enabled: bool) -> Self {
        let AuthPorts {
            ids,
            users,
            sessions,
            hasher,
            breached,
        } = ports;
        Self {
            sign_up: SignUp {
                ids: ids.clone(),
                users: users.clone(),
                hasher: hasher.clone(),
                breached,
                sessions: sessions.clone(),
            },
            log_in: LogIn {
                users: users.clone(),
                hasher,
                sessions: sessions.clone(),
            },
            demo_log_in: DemoLogIn {
                enabled: demo_login_enabled,
                ids,
                users: users.clone(),
                sessions: sessions.clone(),
            },
            log_out: LogOut {
                sessions: sessions.clone(),
            },
            authenticate: Authenticate { sessions, users },
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/signup", post(sign_up))
        .route("/login", post(log_in))
        .route("/logout", post(log_out))
        .route("/me", get(me))
        .route("/demo", get(demo_status))
        .route("/demo-login", post(demo_log_in))
}

fn api_error(err: Error) -> ApiError {
    match err {
        Error::Rejected(err) => err.into(),
        Error::Internal(err) => ApiError::internal(err),
    }
}

fn invalid_request(_: JsonRejection) -> ApiError {
    ApiError::invalid_request()
}

/// The session token the request came with, if any. Its validity is not
/// checked here.
pub struct SessionCookie(pub Option<SessionToken>);

fn session_token(headers: &HeaderMap) -> Option<SessionToken> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, value)| *name == SESSION_COOKIE && !value.is_empty())
        .map(|(_, value)| SessionToken::new(Secret::new(value.to_owned())))
}

impl<S: Send + Sync> FromRequestParts<S> for SessionCookie {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(session_token(&parts.headers)))
    }
}

/// The signed-in user. Rejects with 401 `UNAUTHENTICATED` when the request
/// has no live session.
pub struct AuthUser(pub UserId);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        match MaybeUser::from_request_parts(parts, state).await? {
            MaybeUser(Some(id)) => Ok(Self(id)),
            MaybeUser(None) => Err(AuthError::Unauthenticated.into()),
        }
    }
}

/// The signed-in user, or `None` for anonymous requests and dead sessions.
pub struct MaybeUser(pub Option<UserId>);

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let Some(token) = session_token(&parts.headers) else {
            return Ok(Self(None));
        };
        match state.auth.authenticate.user_id(&token).await {
            Ok(id) => Ok(Self(Some(id))),
            Err(Error::Rejected(_)) => Ok(Self(None)),
            Err(err) => Err(api_error(err)),
        }
    }
}

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DemoLoginRequest {
    account: DemoAccountDto,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum DemoAccountDto {
    Streamer,
    Viewer,
}

impl From<DemoAccountDto> for DemoAccount {
    fn from(dto: DemoAccountDto) -> Self {
        match dto {
            DemoAccountDto::Streamer => Self::Streamer,
            DemoAccountDto::Viewer => Self::Viewer,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserResponse {
    #[serde(with = "crate::json::snowflake_id")]
    id: SnowflakeId,
    username: String,
}

impl From<&User> for UserResponse {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            username: user.username.as_str().to_owned(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DemoStatus {
    enabled: bool,
}

fn session_cookie(token: &SessionToken) -> HeaderValue {
    let cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={}",
        token.expose(),
        ABSOLUTE_TIMEOUT.as_secs()
    );
    HeaderValue::try_from(cookie).expect("base64url tokens are valid header values")
}

fn cleared_cookie() -> HeaderValue {
    HeaderValue::from_static("__Host-session=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0")
}

fn signed_in(status: StatusCode, signed_in: SignedIn) -> Response {
    (
        status,
        [(SET_COOKIE, session_cookie(&signed_in.token))],
        Json(UserResponse::from(&signed_in.user)),
    )
        .into_response()
}

async fn sign_up(
    State(state): State<AppState>,
    client: ClientKey,
    SessionCookie(previous): SessionCookie,
    body: Result<Json<Credentials>, JsonRejection>,
) -> Result<Response, ApiError> {
    state.limits.signup(&client)?;
    let Json(body) = body.map_err(invalid_request)?;
    let result = state
        .auth
        .sign_up
        .run(
            &body.username,
            Secret::new(body.password),
            previous.as_ref(),
        )
        .await
        .map_err(api_error)?;
    Ok(signed_in(StatusCode::CREATED, result))
}

async fn log_in(
    State(state): State<AppState>,
    client: ClientKey,
    SessionCookie(previous): SessionCookie,
    body: Result<Json<Credentials>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(body) = body.map_err(invalid_request)?;
    state.limits.login(&client, &body.username)?;
    let result = state
        .auth
        .log_in
        .run(
            &body.username,
            Secret::new(body.password),
            previous.as_ref(),
        )
        .await
        .map_err(api_error)?;
    Ok(signed_in(StatusCode::OK, result))
}

async fn demo_log_in(
    State(state): State<AppState>,
    client: ClientKey,
    SessionCookie(previous): SessionCookie,
    body: Result<Json<DemoLoginRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    state.limits.demo_login(&client)?;
    let Json(body) = body.map_err(invalid_request)?;
    let result = state
        .auth
        .demo_log_in
        .run(body.account.into(), previous.as_ref())
        .await
        .map_err(api_error)?;
    Ok(signed_in(StatusCode::OK, result))
}

async fn log_out(
    State(state): State<AppState>,
    SessionCookie(token): SessionCookie,
) -> Result<Response, ApiError> {
    state
        .auth
        .log_out
        .run(token.as_ref())
        .await
        .map_err(api_error)?;
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, cleared_cookie())]).into_response())
}

async fn me(
    State(state): State<AppState>,
    SessionCookie(token): SessionCookie,
) -> Result<Json<UserResponse>, ApiError> {
    let token = token.ok_or(AuthError::Unauthenticated)?;
    let user = state
        .auth
        .authenticate
        .current_user(&token)
        .await
        .map_err(api_error)?;
    Ok(Json(UserResponse::from(&user)))
}

async fn demo_status(State(state): State<AppState>) -> Json<DemoStatus> {
    Json(DemoStatus {
        enabled: state.auth.demo_log_in.enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(cookies: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for cookie in cookies {
            headers.append(COOKIE, HeaderValue::from_str(cookie).unwrap());
        }
        headers
    }

    fn token(cookies: &[&str]) -> Option<String> {
        session_token(&headers(cookies)).map(|t| t.expose().to_owned())
    }

    #[test]
    fn session_cookie_is_found_among_others() {
        assert_eq!(token(&["a=1; __Host-session=tok; b=2"]), Some("tok".into()));
        assert_eq!(token(&["a=1", "__Host-session=tok"]), Some("tok".into()));
        assert_eq!(token(&["session=tok; __Host-sessionx=y"]), None);
        assert_eq!(token(&["__Host-session="]), None);
        assert_eq!(token(&[]), None);
    }
}
