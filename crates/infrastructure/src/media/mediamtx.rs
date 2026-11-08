//! MediaMTX as the media server: stream paths, WHIP and playback URLs, and its Control
//! API for kicking publishers and listing the streams on air.

use std::{sync::Arc, time::Duration};

use application::ports::{
    PortError,
    media::{
        IngestEndpoint, IngestRouter, PlaybackKind, PlaybackResolver, PlaybackSource, StreamPath,
        StreamProbe, StreamTerminator,
    },
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

    fn broadcast_of(&self, path: &StreamPath) -> Option<BroadcastId> {
        let id: BroadcastId = path.as_str().strip_prefix("live/")?.parse().ok()?;
        (self.route(id).path == *path).then_some(id)
    }
}

/// Same-origin URLs through Caddy, which forwards `/media/hls/*` and
/// `/media/webrtc/*` to MediaMTX. Reading needs no credential.
pub struct MediaMtxPlaybackResolver;

impl PlaybackResolver for MediaMtxPlaybackResolver {
    /// LL-HLS first: it is plain HTTP, so it plays wherever the page loads,
    /// while WHEP needs a WebRTC connection to the media port.
    fn resolve(&self, path: &StreamPath) -> Vec<PlaybackSource> {
        let path = path.as_str();
        vec![
            PlaybackSource {
                kind: PlaybackKind::LlHls,
                url: format!("/media/hls/{path}/index.m3u8"),
            },
            PlaybackSource {
                kind: PlaybackKind::Whep,
                url: format!("/media/webrtc/{path}/whep"),
            },
        ]
    }
}

/// The Control API, called with a token the API signs itself; MediaMTX hands
/// it back to the API to check.
pub struct MediaMtxControlApi {
    http: reqwest::Client,
    api_url: String,
    key: Arc<PublishKey>,
}

impl MediaMtxControlApi {
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathPage {
    page_count: u32,
    items: Vec<PathItem>,
}

#[derive(Deserialize)]
struct PathItem {
    name: String,
    /// A publisher is sending (`ready` in older versions).
    available: bool,
}

fn port_error(err: reqwest::Error) -> PortError {
    PortError(format!("mediamtx api: {err}"))
}

impl MediaMtxControlApi {
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
impl StreamTerminator for MediaMtxControlApi {
    /// WHIP is the only way in; every other protocol is off in mediamtx.yml.
    async fn kick(&self, path: &StreamPath) -> Result<(), PortError> {
        let token = self.key.control_token()?;
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

#[async_trait]
impl StreamProbe for MediaMtxControlApi {
    async fn live_paths(&self) -> Result<Vec<StreamPath>, PortError> {
        let token = self.key.control_token()?;
        let mut live = Vec::new();
        let mut page = 0;
        loop {
            let paths: PathPage = self
                .http
                .get(format!("{}/v3/paths/list?page={page}", self.api_url))
                .bearer_auth(&token)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(port_error)?
                .json()
                .await
                .map_err(port_error)?;
            live.extend(
                paths
                    .items
                    .into_iter()
                    .filter(|path| path.available)
                    .map(|path| StreamPath::new(path.name)),
            );
            page += 1;
            if page >= paths.page_count {
                return Ok(live);
            }
        }
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
        assert_eq!(MediaMtxIngestRouter.broadcast_of(&endpoint.path), Some(id));
    }

    #[test]
    fn viewers_get_ll_hls_then_whep_through_the_media_prefix() {
        let id = BroadcastId::try_from(112007303372210176).unwrap();
        let sources = MediaMtxPlaybackResolver.resolve(&MediaMtxIngestRouter.route(id).path);
        assert_eq!(
            sources,
            [
                PlaybackSource {
                    kind: PlaybackKind::LlHls,
                    url: "/media/hls/live/112007303372210176/index.m3u8".into(),
                },
                PlaybackSource {
                    kind: PlaybackKind::Whep,
                    url: "/media/webrtc/live/112007303372210176/whep".into(),
                },
            ]
        );
    }

    /// Captured from the Control API of the MediaMTX image in compose while
    /// one WHIP publisher was on air.
    #[test]
    fn a_paths_list_from_mediamtx_1_21_3_names_the_path_on_air() {
        let page: PathPage =
            serde_json::from_str(include_str!("testdata/mediamtx-1.21.3-paths-list.json")).unwrap();
        assert_eq!(page.page_count, 1);
        let on_air: Vec<_> = page
            .items
            .iter()
            .filter(|path| path.available)
            .map(|path| path.name.as_str())
            .collect();
        assert_eq!(on_air, ["live/112845401270296576"]);
    }

    #[test]
    fn only_paths_the_router_would_produce_name_a_broadcast() {
        for raw in [
            "live/",
            "live/abc",
            "live/-1",
            "live/+7",
            "live/007",
            "live/7/x",
            "live/7 ",
            "other/7",
            "7",
            "",
            "live/99999999999999999999",
        ] {
            assert_eq!(
                MediaMtxIngestRouter.broadcast_of(&StreamPath::new(raw.into())),
                None,
                "{raw:?}"
            );
        }
        assert_eq!(
            MediaMtxIngestRouter.broadcast_of(&StreamPath::new("live/7".into())),
            BroadcastId::try_from(7).ok()
        );
    }
}
