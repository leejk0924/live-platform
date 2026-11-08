//! MediaMTX as the media server: stream paths, WHIP URLs and kicking a
//! publisher through its Control API.

use std::{sync::Arc, time::Duration};

use application::ports::{
    PortError,
    media::{IngestEndpoint, IngestRouter, StreamPath, StreamTerminator},
};
use async_trait::async_trait;
use domain::broadcast::BroadcastId;
use reqwest::StatusCode;
use serde::Deserialize;

use super::jwt::PublishKey;

/// One MediaMTX node behind the same origin as the web app; Caddy forwards
/// `/media/webrtc/*` to it.
pub struct MediaMtxIngestRouter;

impl IngestRouter for MediaMtxIngestRouter {
    fn route(&self, broadcast: BroadcastId) -> IngestEndpoint {
        IngestEndpoint {
            path: StreamPath::new(format!("live/{broadcast}")),
            whip_url: format!("/media/webrtc/live/{broadcast}/whip"),
        }
    }
}

/// Kicks a path's publisher through the Control API, authenticating with a
/// token signed by the same key MediaMTX already trusts.
pub struct MediaMtxStreamTerminator {
    http: reqwest::Client,
    api_url: String,
    key: Arc<PublishKey>,
}

impl MediaMtxStreamTerminator {
    /// `api_url` is the Control API base, such as `http://mediamtx:9997`.
    pub fn new(api_url: String, key: Arc<PublishKey>) -> Result<Self, PortError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|err| PortError(format!("http client: {err}")))?;
        Ok(Self { http, api_url, key })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionPage {
    page_count: u32,
    items: Vec<Session>,
}

#[derive(Deserialize)]
struct Session {
    id: String,
    path: String,
}

fn port_error(err: reqwest::Error) -> PortError {
    PortError(format!("mediamtx api: {err}"))
}

impl MediaMtxStreamTerminator {
    /// Every WebRTC session on `path`. A WHIP session joins this list as soon
    /// as its offer is answered, while the path itself names it as the source
    /// only once ICE connects.
    async fn sessions_on(&self, path: &StreamPath, token: &str) -> Result<Vec<String>, PortError> {
        let mut ids = Vec::new();
        let mut page = 0;
        loop {
            let sessions: SessionPage = self
                .http
                .get(format!(
                    "{}/v3/webrtc/sessions/list?page={page}",
                    self.api_url
                ))
                .bearer_auth(token)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(port_error)?
                .json()
                .await
                .map_err(port_error)?;
            ids.extend(
                sessions
                    .items
                    .into_iter()
                    .filter(|session| session.path == path.as_str())
                    .map(|session| session.id),
            );
            page += 1;
            if page >= sessions.page_count {
                return Ok(ids);
            }
        }
    }
}

#[async_trait]
impl StreamTerminator for MediaMtxStreamTerminator {
    /// WHIP is the only way in; every other protocol is off in mediamtx.yml.
    async fn kick(&self, path: &StreamPath) -> Result<(), PortError> {
        let token = self.key.api_token()?;
        for id in self.sessions_on(path, &token).await? {
            let response = self
                .http
                .post(format!("{}/v3/webrtc/sessions/kick/{id}", self.api_url))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(port_error)?;
            // Not found: the session closed after the list was read.
            if response.status() != StatusCode::NOT_FOUND {
                response.error_for_status().map_err(port_error)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broadcast_publishes_to_its_own_path_through_the_media_prefix() {
        let id = BroadcastId::try_from(112007303372210176).unwrap();
        let endpoint = MediaMtxIngestRouter.route(id);
        assert_eq!(endpoint.path.as_str(), "live/112007303372210176");
        assert_eq!(
            endpoint.whip_url,
            "/media/webrtc/live/112007303372210176/whip"
        );
    }
}
