//! The OpenAPI description of `/api`, generated from the handlers. The
//! frontend generates its client types from the committed copy in
//! `web/src/api/openapi.json`, and a test keeps that copy current.

use axum::Json;
use utoipa::{
    Modify, OpenApi,
    openapi::{
        self,
        security::{ApiKey, ApiKeyValue, SecurityScheme},
    },
};

use crate::{auth, broadcast, live};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "live-platform API",
        version = "0.1.0",
        description = "Errors are always `{code, message, traceId}` (`ErrorBody`). \
            Every POST, PUT, PATCH and DELETE must send an allowed `Origin` and \
            `X-Requested-With: live-platform`, or it is rejected with 403 `CSRF_REJECTED`. \
            Ids are decimal strings.",
    ),
    paths(
        auth::sign_up,
        auth::log_in,
        auth::log_out,
        auth::me,
        auth::demo_status,
        auth::demo_log_in,
        broadcast::list,
        broadcast::get_one,
        broadcast::create,
        broadcast::update,
        broadcast::delete,
        live::start,
        live::stop,
        live::jwks,
    ),
    modifiers(&SessionCookie),
    tags(
        (name = "auth", description = "Accounts and sessions"),
        (name = "broadcasts", description = "Broadcast registration, browsing, going live and ending"),
    ),
)]
pub struct ApiDoc;

struct SessionCookie;

impl Modify for SessionCookie {
    fn modify(&self, openapi: &mut openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                auth::SESSION_COOKIE,
                "Set by signup, login and demo-login; HttpOnly, so browsers send it themselves.",
            ))),
        );
    }
}

pub async fn spec() -> Json<openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf};

    use super::*;

    /// `UPDATE_OPENAPI=1 cargo test -p api openapi` rewrites the snapshot.
    #[test]
    fn committed_openapi_json_matches_the_handlers() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/src/api/openapi.json");
        let current = ApiDoc::openapi().to_pretty_json().unwrap() + "\n";
        if env::var_os("UPDATE_OPENAPI").is_some_and(|v| v == "1") {
            fs::write(&path, &current).unwrap();
            return;
        }
        let committed = fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            committed == current,
            "{} is stale; regenerate it with UPDATE_OPENAPI=1 cargo test -p api openapi",
            path.display()
        );
    }
}
