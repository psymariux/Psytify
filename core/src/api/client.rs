//! HTTP client for the Psytify backend and for the storage's signed URLs.
//!
//! Every request has a timeout. Signed URLs are never logged nor returned inside
//! an error: `AppError` keeps only the status and the code.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::StatusCode;

use super::transfer::{Progress, TempFile};
use super::update;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use super::models::DownloadQuality;
use super::models::{
    Album, AlbumSummary, ApiErrorBody, ArtistPage, DiscoverImport, DiscoverPage, LibraryPage, Lyrics, Me,
    MetaArtistPage, MetaSearch, Page, Quality, SearchResults, SignedUrl, SourceKind, SourcePlaylistPage,
    Track,
};
use super::models::{MetaArtistPageDto, MetaListPageDto, MetaSearchDto, OpenedArtist};
use crate::error::AppError;
use crate::player::stream::{Next, PREFETCH_BYTES, StreamBuffer};

/// Overall timeout for the catalog calls.
pub const API_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall timeout for downloading a track entirely into memory (tests against the live server).
#[cfg(test)]
pub const AUDIO_TIMEOUT: Duration = Duration::from_secs(120);
/// A long download has no total timeout, but it is interrupted if no data arrives.
pub const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The import reads the metadata from Archive: it may take a few seconds more.
const IMPORT_TIMEOUT: Duration = Duration::from_secs(30);
/// Search on the external sources: the server queries three sources in parallel.
const SOURCE_SEARCH_TIMEOUT: Duration = Duration::from_secs(45);
/// Importing from an external source downloads the audio to the server (the server stops at 120 seconds).
const SOURCE_IMPORT_TIMEOUT: Duration = Duration::from_secs(130);
/// Official catalog: the server queries Deezer (several pages for an artist's discography).
const META_TIMEOUT: Duration = Duration::from_secs(25);
/// The first `video-url` of a track downloads the music video to the server (which stops at 120 seconds).
#[cfg(feature = "video")]
const VIDEO_TIMEOUT: Duration = Duration::from_secs(130);
/// Preparing a music video answers at once: the download stays on the server.
const PREPARE_TIMEOUT: Duration = Duration::from_secs(15);
/// Covers: timeout and maximum size of an image.
const COVER_TIMEOUT: Duration = Duration::from_secs(20);
/// The backend waits for LRCLIB (up to 10 s per search) before answering.
const LYRICS_TIMEOUT: Duration = Duration::from_secs(25);
pub const MAX_COVER_BYTES: usize = 4 * 1024 * 1024;
/// Redirects followed at most for a cover (archive.org sends to a storage node).
const MAX_COVER_REDIRECTS: usize = 5;
/// Beyond this size a track is not loaded into memory for listening.
pub const MAX_STREAM_BYTES: u64 = 600 * 1024 * 1024;
/// Attempts of a `Range` request after a jump, before giving the track up as lost.
const RANGE_ATTEMPTS: usize = 2;
/// Resolved URLs remembered at most: beyond it one starts again from scratch.
const MAX_RESOLVED: usize = 256;
/// How many times a download that fell halfway restarts from where it had reached.
const MAX_REOPEN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Recent,
    Title,
}

impl Sort {
    fn as_str(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Title => "title",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackQuery {
    pub q: Option<String>,
    pub sort: Sort,
    pub limit: u32,
    pub offset: u32,
}

/// Reply of `download-url`: the ready URL, or the requested version is still being prepared on the
/// server (the re-encoding) and must be requested again shortly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadReply {
    Ready(SignedUrl),
    Preparing(Duration),
}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    download: reqwest::Client,
    /// Only covers, requested from the backend. The check on the image's host (`is_allowed_cover_url`)
    /// stays: the backend repeats it, but the client never asks for anything else.
    covers: reqwest::Client,
    base: Arc<str>,
    /// Listen URL → final URL after the redirects, for the session. archive.org sends each
    /// request to a storage node and the redirect costs about 2.7 seconds: listening again to a
    /// track or jumping inside it goes straight to the node. In memory only, never in logs.
    resolved: Arc<Mutex<HashMap<String, reqwest::Url>>>,
}

/// A cover URL is downloaded only over https from `archive.org` or one of its subdomains
/// (`ia800608.us.archive.org`), or from Deezer's image CDN (`cdn-images.dzcdn.net`), from
/// which the backend takes the official covers of YouTube tracks and the artists' photos. These are
/// the only allowed exceptions to the URLs resolved by the backend, and only for images.
pub fn is_allowed_cover_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|host| {
            host == "archive.org"
                || host.ends_with(".archive.org")
                || host.ends_with(".dzcdn.net")
                // The profiles' avatars.
                || host == "img.clerk.com"
        })
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient").field("base", &self.base).finish_non_exhaustive()
    }
}

/// Host and port of a backend address (as [`ApiClient::host`] shows them), after the same check
/// as [`ApiClient::new`]: `http` or `https` with a host. It serves a frontend that wants to check an
/// address typed by hand without building the HTTP clients.
pub fn base_host(base_url: &str) -> Result<String, AppError> {
    let base = base_url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(base).map_err(|_| AppError::InvalidBaseUrl(base.into()))?;
    match parsed.host_str() {
        Some(host) if matches!(parsed.scheme(), "http" | "https") => Ok(match parsed.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        }),
        _ => Err(AppError::InvalidBaseUrl(base.into())),
    }
}

impl ApiClient {
    pub fn new(base_url: &str) -> Result<Self, AppError> {
        let base = base_url.trim().trim_end_matches('/');
        base_host(base)?;
        let user_agent = concat!("psytify/", env!("CARGO_PKG_VERSION"));
        let http = reqwest::Client::builder()
            .timeout(API_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(user_agent)
            .build()
            .map_err(|_| AppError::Network)?;
        let download = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(DOWNLOAD_READ_TIMEOUT)
            .user_agent(user_agent)
            .build()
            .map_err(|_| AppError::Network)?;
        let covers = reqwest::Client::builder()
            .timeout(COVER_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= MAX_COVER_REDIRECTS || !is_allowed_cover_url(attempt.url()) {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| AppError::Network)?;
        Ok(Self { http, download, covers, base: base.into(), resolved: Arc::default() })
    }

    /// The backend address, without a trailing slash (whoever keeps a client cached compares it with
    /// the one chosen in the settings).
    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// Backend host and port, to show in the interface.
    pub fn host(&self) -> String {
        reqwest::Url::parse(&self.base)
            .ok()
            .and_then(|url| {
                url.host_str().map(|h| match url.port() {
                    Some(port) => format!("{h}:{port}"),
                    None => h.to_owned(),
                })
            })
            .unwrap_or_default()
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, AppError> {
        self.get_json_within(path, query, API_TIMEOUT).await
    }

    /// As `get_json`, with its own maximum time: the backend may wait for an external service
    /// (LRCLIB) longer than a catalog request.
    async fn get_json_within<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        timeout: Duration,
    ) -> Result<T, AppError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .query(query)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        let status = response.status();
        if status.is_success() {
            return response.json::<T>().await.map_err(|e| AppError::from_http(&e));
        }
        let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
        Err(api_error(status, code))
    }

    pub async fn tracks(&self, query: TrackQuery) -> Result<Page<Track>, AppError> {
        let mut params = vec![
            ("sort", query.sort.as_str().to_owned()),
            ("limit", query.limit.to_string()),
            ("offset", query.offset.to_string()),
        ];
        if let Some(q) = query.q {
            params.push(("q", q));
        }
        self.get_json("/v1/catalog/tracks", &params).await
    }

    /// Search in the catalog: main result, tracks, artists and albums.
    pub async fn search(&self, q: &str) -> Result<SearchResults, AppError> {
        self.get_json("/v1/catalog/search", &[("q", q.to_owned())]).await
    }

    pub async fn albums(&self, sort: Sort, limit: u32, offset: u32) -> Result<Page<AlbumSummary>, AppError> {
        let params = [
            ("sort", sort.as_str().to_owned()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        self.get_json("/v1/catalog/albums", &params).await
    }

    /// Searches the official catalog: tracks, artists, albums and playlists.
    pub async fn meta_search(&self, q: &str) -> Result<MetaSearch, AppError> {
        let dto: MetaSearchDto =
            self.get_json_within("/v1/meta/search", &[("q", q.to_owned())], META_TIMEOUT).await?;
        Ok(dto.into())
    }

    /// The artist with this name: popular tracks and the whole discography.
    pub async fn meta_artist_named(&self, name: &str) -> Result<MetaArtistPage, AppError> {
        let dto: MetaArtistPageDto =
            self.get_json_within("/v1/meta/artists", &[("name", name.to_owned())], META_TIMEOUT).await?;
        Ok(dto.into())
    }

    /// Brings a catalog artist into the server and returns its identifier (UUID).
    pub async fn open_meta_artist(&self, id: u64) -> Result<Uuid, AppError> {
        let response = self
            .http
            .post(format!("{}/v1/meta/artists/{id}/open", self.base))
            .timeout(META_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        let status = response.status();
        if status.is_success() {
            let opened = response.json::<OpenedArtist>().await.map_err(|e| AppError::from_http(&e))?;
            return Ok(opened.id);
        }
        let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
        Err(api_error(status, code))
    }

    /// Catalog album or playlist (`album/123`, `playlist/456`) with its tracks.
    async fn meta_list(&self, id: &str) -> Result<SourcePlaylistPage, AppError> {
        let (kind, number) = id.split_once('/').ok_or(AppError::InvalidResponse)?;
        if !matches!(kind, "album" | "playlist")
            || number.is_empty()
            || !number.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(AppError::InvalidResponse);
        }
        let dto: MetaListPageDto =
            self.get_json_within(&format!("/v1/meta/{kind}s/{number}"), &[], META_TIMEOUT).await?;
        Ok(dto.into_page(kind))
    }

    /// The tracks of an external playlist. The identifier is the one received from the server.
    pub async fn source_playlist(
        &self,
        source: SourceKind,
        id: &str,
    ) -> Result<SourcePlaylistPage, AppError> {
        if source == SourceKind::Deezer {
            return self.meta_list(id).await;
        }
        if !is_opaque_id(id) {
            return Err(AppError::InvalidResponse);
        }
        let response = self
            .http
            .get(format!("{}/v1/sources/playlists/{}/{}", self.base, source.as_str(), id))
            .timeout(SOURCE_SEARCH_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        let status = response.status();
        if status.is_success() {
            return response.json::<SourcePlaylistPage>().await.map_err(|e| AppError::from_http(&e));
        }
        let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
        Err(api_error(status, code))
    }

    /// Brings an external search result into the catalog and returns the track. `source` is one
    /// of `youtube`, `soundcloud`, `bandcamp`; the identifier is the one received from the server.
    /// With `video` (only for the official catalog) the server downloads the music video along with the audio.
    pub async fn import_source(
        &self,
        source: SourceKind,
        id: &str,
        video: bool,
    ) -> Result<DiscoverImport, AppError> {
        let valid = match source {
            SourceKind::Deezer => !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
            SourceKind::YouTube => {
                id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            }
            SourceKind::SoundCloud | SourceKind::Bandcamp => is_opaque_id(id),
        };
        if !valid {
            return Err(AppError::InvalidResponse);
        }
        let response = self
            .http
            .post(match source {
                SourceKind::Deezer if video => {
                    format!("{}/v1/meta/tracks/{id}/resolve?video=true", self.base)
                }
                SourceKind::Deezer => format!("{}/v1/meta/tracks/{id}/resolve", self.base),
                _ => format!("{}/v1/sources/{}/{}", self.base, source.as_str(), id),
            })
            .timeout(SOURCE_IMPORT_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        let status = response.status();
        if status.is_success() {
            return response.json::<DiscoverImport>().await.map_err(|e| AppError::from_http(&e));
        }
        let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
        Err(api_error(status, code))
    }

    /// Bytes of a cover, from the backend (`/v1/cover`): the client opens no connections to Deezer
    /// or Internet Archive. The original address is checked anyway before asking for it, at most
    /// `MAX_COVER_BYTES`.
    pub async fn cover(&self, url: &str) -> Result<Vec<u8>, AppError> {
        let parsed = reqwest::Url::parse(url).map_err(|_| AppError::InvalidResponse)?;
        if !is_allowed_cover_url(&parsed) {
            return Err(AppError::InvalidResponse);
        }
        let mut response = self
            .covers
            .get(format!("{}/v1/cover", self.base))
            .query(&[("url", parsed.as_str())])
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        if !response.status().is_success() {
            return Err(api_error(response.status(), "storage".into()));
        }
        if response.content_length().is_some_and(|len| len > MAX_COVER_BYTES as u64) {
            return Err(AppError::InvalidResponse);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| AppError::from_http(&e))? {
            if bytes.len() + chunk.len() > MAX_COVER_BYTES {
                return Err(AppError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    pub async fn album(&self, id: Uuid) -> Result<Album, AppError> {
        self.get_json(&format!("/v1/catalog/albums/{id}"), &[]).await
    }

    /// A single catalog track: the jam uses it for the tracks of the shared queue it does not have.
    pub async fn track(&self, id: Uuid) -> Result<Track, AppError> {
        self.get_json(&format!("/v1/catalog/tracks/{id}"), &[]).await
    }

    pub async fn stream_url(&self, id: Uuid, quality: Quality) -> Result<SignedUrl, AppError> {
        let signed = self
            .get_json(&format!("/v1/tracks/{id}/stream-url"), &[("quality", quality.as_str().to_owned())])
            .await?;
        Ok(self.absolute(signed))
    }

    /// A track or music video just imported is served by the backend itself, from its disk: the URL is
    /// relative to its origin.
    fn absolute(&self, mut signed: SignedUrl) -> SignedUrl {
        if signed.url.starts_with("/v1/") {
            signed.url = format!("{}{}", self.base, signed.url);
        }
        signed
    }

    /// Track lyrics. `found: false` if there are none; `503` if the backend cannot reach LRCLIB.
    pub async fn lyrics(&self, id: Uuid) -> Result<Lyrics, AppError> {
        self.get_json_within(&format!("/v1/tracks/{id}/lyrics"), &[], LYRICS_TIMEOUT).await
    }

    /// Signed URL of the music video; `404` if the track has none.
    #[cfg(feature = "video")]
    pub async fn video_url(&self, id: Uuid) -> Result<SignedUrl, AppError> {
        let signed = self.get_json_within(&format!("/v1/tracks/{id}/video-url"), &[], VIDEO_TIMEOUT).await?;
        Ok(self.absolute(signed))
    }

    /// Asks the server to prepare the music video of a track that will play soon: it answers at once.
    pub async fn prepare_video(&self, id: Uuid) -> Result<(), AppError> {
        let response = self
            .http
            .post(format!("{}/v1/tracks/{id}/video/prepare", self.base))
            .timeout(PREPARE_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
        Err(api_error(status, code))
    }

    /// A request with the login (profiles, friends, activity). `None` for replies without a body
    /// (`204`); `401` becomes `AuthExpired`, the server's messages stay those of `api_error`.
    pub(crate) async fn authed<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
    ) -> Result<Option<T>, AppError> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token)
            .query(query)
            .timeout(API_TIMEOUT);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|e| AppError::from_http(&e))?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(None),
            s if s.is_success() => response.json::<T>().await.map(Some).map_err(|e| AppError::from_http(&e)),
            StatusCode::UNAUTHORIZED => Err(AppError::AuthExpired),
            s => {
                let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
                Err(api_error(s, code))
            }
        }
    }

    /// The user of the Clerk token. `401` means an expired or revoked token.
    pub async fn me(&self, access_token: &str) -> Result<Me, AppError> {
        let response = self
            .http
            .get(format!("{}/v1/me", self.base))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        match response.status() {
            s if s.is_success() => response.json::<Me>().await.map_err(|e| AppError::from_http(&e)),
            StatusCode::UNAUTHORIZED => Err(AppError::AuthExpired),
            s => {
                let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
                Err(api_error(s, code))
            }
        }
    }

    pub async fn download_url(&self, id: Uuid, quality: DownloadQuality) -> Result<DownloadReply, AppError> {
        let response = self
            .http
            .get(format!("{}/v1/tracks/{id}/download-url", self.base))
            .query(&[("quality", quality.as_str())])
            .timeout(API_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        let status = response.status();
        // 202: the server is re-encoding the requested version; `Retry-After` says when to retry.
        if status == StatusCode::ACCEPTED {
            let seconds = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(2);
            return Ok(DownloadReply::Preparing(Duration::from_secs(seconds.clamp(1, 10))));
        }
        if status.is_success() {
            let signed = response.json::<SignedUrl>().await.map_err(|e| AppError::from_http(&e))?;
            return Ok(DownloadReply::Ready(self.absolute(signed)));
        }
        let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
        Err(api_error(status, code))
    }

    /// Opens the progressive listening: it checks the state at once (401 and 403 mean the URL has expired),
    /// then a task keeps downloading into the buffer. It returns as soon as there are bytes to start with.
    ///
    /// The file is requested with `Range: bytes=0-`: if the server answers 206, the buffer knows the
    /// length and a far seek restarts the download from that point (see `player::stream`).
    #[cfg(test)]
    pub async fn open_stream(&self, url: &SignedUrl) -> Result<StreamBuffer, AppError> {
        self.open_stream_held(url, None).await
    }

    /// As [`Self::open_stream`], but with `hold` the download stops after those first bytes and
    /// restarts with `StreamBuffer::release`: it keeps the head of a track ready that
    /// may be listened to, without downloading all of it.
    pub async fn open_stream_held(
        &self,
        url: &SignedUrl,
        hold: Option<usize>,
    ) -> Result<StreamBuffer, AppError> {
        let (response, target) = self.first_response(&url.url).await?;
        let buffer = StreamBuffer::new();
        if let Some(hold) = hold {
            buffer.hold_at(hold);
        }
        let total = match response.status() {
            StatusCode::PARTIAL_CONTENT => content_range(&response).map(|(_, total)| total),
            _ => response.content_length(),
        };
        if let Some(total) = total {
            if total > MAX_STREAM_BYTES {
                return Err(AppError::AudioDecode);
            }
            let ranges = response.status() == StatusCode::PARTIAL_CONTENT;
            buffer.set_total(usize::try_from(total).map_err(|_| AppError::AudioDecode)?, ranges);
        }
        tokio::spawn(download_into(self.download.clone(), target, response, buffer.clone()));
        if buffer.ready(PREFETCH_BYTES).await { Ok(buffer) } else { Err(AppError::Network) }
    }

    /// First reply of the stream and final URL to use for the jumps. It tries the node already
    /// resolved first; if it does not answer, it goes back to the original URL.
    async fn first_response(&self, original: &str) -> Result<(reqwest::Response, reqwest::Url), AppError> {
        let cached = self.resolved.lock().ok().and_then(|map| map.get(original).cloned());
        if let Some(node) = cached {
            match self.range_request(node.clone(), 0).await {
                Ok(response) if response.status().is_success() => return Ok((response, node)),
                _ => {
                    if let Ok(mut map) = self.resolved.lock() {
                        map.remove(original);
                    }
                }
            }
        }
        let parsed = reqwest::Url::parse(original).map_err(|_| AppError::InvalidResponse)?;
        let response = self.range_request(parsed.clone(), 0).await.map_err(|e| AppError::from_http(&e))?;
        check_storage_status(response.status())?;
        let target = response.url().clone();
        if target != parsed
            && let Ok(mut map) = self.resolved.lock()
        {
            if map.len() >= MAX_RESOLVED {
                map.clear();
            }
            map.insert(original.to_owned(), target.clone());
        }
        Ok((response, target))
    }

    async fn range_request(&self, url: reqwest::Url, from: usize) -> reqwest::Result<reqwest::Response> {
        range_request(&self.download, url, from).await
    }

    /// Searches Internet Archive for music that is not in the catalog.
    pub async fn discover(&self, q: &str) -> Result<DiscoverPage, AppError> {
        self.get_json("/v1/discover", &[("q", q.to_owned())]).await
    }

    /// A page (from 1) of the albums with a free licence of an Archive library.
    pub async fn library(&self, identifier: &str, page: u32) -> Result<LibraryPage, AppError> {
        if !is_archive_identifier(identifier) {
            return Err(AppError::InvalidResponse);
        }
        self.get_json(&format!("/v1/libraries/{identifier}"), &[("page", page.to_string())]).await
    }

    /// Brings an Archive item into the catalog and returns its tracks, ready to play.
    pub async fn import_item(&self, identifier: &str) -> Result<DiscoverImport, AppError> {
        if !is_archive_identifier(identifier) {
            return Err(AppError::InvalidResponse);
        }
        let response = self
            .http
            .post(format!("{}/v1/discover/{identifier}/import", self.base))
            .timeout(IMPORT_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        let status = response.status();
        if status.is_success() {
            return response.json::<DiscoverImport>().await.map_err(|e| AppError::from_http(&e));
        }
        let code = response.json::<ApiErrorBody>().await.map(|b| b.error).unwrap_or_default();
        Err(api_error(status, code))
    }

    pub async fn artist(&self, id: Uuid) -> Result<ArtistPage, AppError> {
        self.get_json(&format!("/v1/catalog/artists/{id}"), &[]).await
    }

    /// Scarica in memoria i byte di un URL firmato. 401 e 403 indicano un URL scaduto.
    #[cfg(test)]
    pub async fn fetch_audio(&self, url: &SignedUrl) -> Result<Vec<u8>, AppError> {
        let response = self
            .http
            .get(&url.url)
            .timeout(AUDIO_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::from_http(&e))?;
        check_storage_status(response.status())?;
        if response.content_length().is_some_and(|len| len > MAX_STREAM_BYTES) {
            return Err(AppError::AudioDecode);
        }
        let bytes = response.bytes().await.map_err(|e| AppError::from_http(&e))?;
        Ok(bytes.to_vec())
    }

    /// The app's latest release, as the backend reads it.
    pub async fn latest_release(&self) -> Result<update::Release, AppError> {
        self.get_json("/v1/app/release", &[]).await
    }

    /// Downloads a file of the latest release from the backend into the folder `dir` and returns the
    /// path. The name comes from the server and ends up in a path and in a URL: if it is not a simple file
    /// name (`update::safe_asset_name`) the reply is not valid. A file with the same name
    /// (the same release downloaded twice) is replaced.
    pub async fn download_release(
        &self,
        asset: &update::Asset,
        dir: &Path,
        progress: &Progress,
    ) -> Result<PathBuf, AppError> {
        let name = update::safe_asset_name(&asset.name).ok_or(AppError::InvalidResponse)?;
        tokio::fs::create_dir_all(dir).await.map_err(|_| AppError::Io(dir.display().to_string()))?;
        let fetched =
            self.fetch_into(&format!("{}/v1/app/release/assets/{name}", self.base), dir, progress).await?;
        fetched.temp.publish(dir, name, true).await.map_err(|_| AppError::Io(dir.display().to_string()))
    }

    /// Scarica `url` a blocchi in un file temporaneo unico dentro `dir`, contando i byte
    /// (`progress`) and computing the SHA-256 while it writes. If the connection falls halfway and the
    /// server had said the length, it restarts from where it had reached (`Range`), a few times. The
    /// file stays temporary: whoever wants it publishes it with `TempFile::publish`, otherwise it is
    /// removed by itself (error, cancel).
    pub(crate) async fn fetch_into(
        &self,
        url: &str,
        dir: &Path,
        progress: &Progress,
    ) -> Result<Fetched, AppError> {
        let io_error = |_| AppError::Io(dir.display().to_string());
        let mut response = self.download.get(url).send().await.map_err(|e| AppError::from_http(&e))?;
        check_storage_status(response.status())?;
        let total = response.content_length();
        progress.begin(total);
        let (temp, mut file) = TempFile::create(dir).await.map_err(io_error)?;
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        let mut reopened = 0;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    hasher.update(&chunk);
                    file.write_all(&chunk).await.map_err(io_error)?;
                    written += chunk.len() as u64;
                    progress.advance(chunk.len() as u64);
                }
                Ok(None) => break,
                Err(err) => {
                    let again = if total.is_some() && reopened < MAX_REOPEN {
                        reopened += 1;
                        self.resume(url, written).await
                    } else {
                        None
                    };
                    response = again.ok_or_else(|| AppError::from_http(&err))?;
                }
            }
        }
        file.flush().await.map_err(io_error)?;
        drop(file);
        // A reply closed before the announced length without an error is not a whole file.
        if total.is_some_and(|total| total != written) {
            return Err(AppError::Network);
        }
        Ok(Fetched { temp, size: written, sha256: crate::offline::hex(&hasher.finalize()) })
    }

    /// Reopens `url` from `from`: only if the server answers `206` from exactly that byte.
    async fn resume(&self, url: &str, from: u64) -> Option<reqwest::Response> {
        let parsed = reqwest::Url::parse(url).ok()?;
        let response = range_request(&self.download, parsed, usize::try_from(from).ok()?).await.ok()?;
        let starts_here = content_range(&response).is_some_and(|(start, _)| start == from);
        (response.status() == StatusCode::PARTIAL_CONTENT && starts_here).then_some(response)
    }
}

/// A downloaded file, still temporary, with its weight and its SHA-256 (hexadecimal).
#[derive(Debug)]
pub(crate) struct Fetched {
    pub(crate) temp: TempFile,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

async fn range_request(
    client: &reqwest::Client,
    url: reqwest::Url,
    from: usize,
) -> reqwest::Result<reqwest::Response> {
    client.get(url).header(reqwest::header::RANGE, format!("bytes={from}-")).send().await
}

/// Start and total length from `Content-Range: bytes 100-199/1000`.
fn content_range(response: &reqwest::Response) -> Option<(u64, u64)> {
    let value = response.headers().get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    parse_content_range(value)
}

fn parse_content_range(value: &str) -> Option<(u64, u64)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// Writes the response in progress into the buffer and, when the buffer asks, opens another one from a
/// another point of the file (a distant seek, then the holes left).
async fn download_into(
    client: reqwest::Client,
    target: reqwest::Url,
    mut response: reqwest::Response,
    writer: StreamBuffer,
) {
    let mut offset = match response.status() {
        StatusCode::PARTIAL_CONTENT => {
            content_range(&response).and_then(|(start, _)| usize::try_from(start).ok())
        }
        _ => Some(0),
    };
    let mut received: u64 = 0;
    // A response dropped halfway (the server closes a connection idle for a while) is reopened from the
    // point where it had arrived, a few times.
    let mut reopened = 0;
    loop {
        let Some(at) = offset else {
            writer.fail();
            return;
        };
        // Header ready and no listening in progress: one waits, without using bandwidth and without polling (it
        // wakes on `release`, `cancel` or when a reader goes beyond the head).
        writer.wait_released().await;
        if writer.is_cancelled() {
            return;
        }
        let arrival = tokio::select! {
            chunk = response.chunk() => Some(chunk),
            () = writer.jump_requested() => None,
        };
        let next = match arrival {
            // A reader asks for a jump: an empty block tells the buffer where to go.
            None => match writer.write_at(at, &[]) {
                Next::Continue(_) => continue,
                other => other,
            },
            Some(Ok(Some(chunk))) => {
                received += chunk.len() as u64;
                // Without an announced length the limit is checked here; with jumps a byte can
                // arrive twice, so the count has a margin.
                if received > 2 * MAX_STREAM_BYTES {
                    writer.fail();
                    return;
                }
                match writer.write_at(at, &chunk) {
                    Next::Continue(_) => {
                        offset = Some(at + chunk.len());
                        continue;
                    }
                    other => other,
                }
            }
            Some(Ok(None)) => writer.response_ended(at),
            Some(Err(_)) if reopened < MAX_REOPEN && writer.can_range() => {
                reopened += 1;
                Next::Jump(at)
            }
            Some(Err(_)) => {
                writer.fail();
                return;
            }
        };
        let Next::Jump(from) = next else { return };
        let mut attempt = 0;
        response = loop {
            attempt += 1;
            match range_request(&client, target.clone(), from).await {
                Ok(fresh) if fresh.status() == StatusCode::PARTIAL_CONTENT => break fresh,
                _ if attempt < RANGE_ATTEMPTS && !writer.is_cancelled() => continue,
                _ => {
                    writer.fail();
                    return;
                }
            }
        };
        offset = content_range(&response).and_then(|(start, _)| usize::try_from(start).ok());
    }
}

fn check_storage_status(status: StatusCode) -> Result<(), AppError> {
    match status {
        s if s.is_success() => Ok(()),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(AppError::UrlExpired),
        s => Err(api_error(s, "storage".into())),
    }
}

/// Archive identifier: it comes from the server but ends up in a path, so it is checked again.
fn is_archive_identifier(identifier: &str) -> bool {
    identifier.len() <= 100
        && identifier.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && identifier.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Message in Italian for each stable code of docs/BACKEND.md.
/// An external source identifier sits in a single path segment: letters, digits, `-`
/// and `_`, at most 200 characters.
fn is_opaque_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn api_error(status: StatusCode, code: String) -> AppError {
    let message = match code.as_str() {
        "not_found" => "Contenuto non disponibile.",
        "not_downloadable" => "Questo brano non è scaricabile.",
        "rate_limited" => "Troppe richieste. Riprova tra qualche secondo.",
        "storage_unavailable" | "storage" => "L'archivio audio non risponde. Riprova.",
        "timeout" => "Il server non ha risposto in tempo.",
        "bad_request" => "Richiesta non valida.",
        "upstream_unavailable" => {
            "La fonte esterna (Internet Archive o YouTube) non risponde. Riprova tra poco."
        }
        "conflict" => "Questo contenuto è già in importazione. Riprova tra poco.",
        "validation_failed" => "Questo contenuto non si può ascoltare (licenza non libera o audio assente).",
        _ => "Il server ha risposto con un errore.",
    };
    AppError::Api { status: status.as_u16(), code, message: message.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_must_be_http() {
        assert!(matches!(ApiClient::new("ftp://example.org"), Err(AppError::InvalidBaseUrl(_))));
    }

    #[test]
    fn the_host_of_an_address_is_checked_like_the_client_does() {
        assert_eq!(base_host("https://api.example.org/").unwrap(), "api.example.org");
        assert_eq!(base_host(" http://10.0.0.2:8080 ").unwrap(), "10.0.0.2:8080");
        assert!(matches!(base_host("ftp://example.org"), Err(AppError::InvalidBaseUrl(_))));
        assert!(matches!(base_host("non è un indirizzo"), Err(AppError::InvalidBaseUrl(_))));
        assert_eq!(ApiClient::new("http://10.0.0.2:8080").unwrap().host(), "10.0.0.2:8080");
    }

    #[test]
    fn trailing_slash_is_removed() {
        let client = ApiClient::new("https://api.example.org/").unwrap();
        assert_eq!(&*client.base, "https://api.example.org");
    }

    #[test]
    fn host_includes_port() {
        assert_eq!(ApiClient::new("http://127.0.0.1:8080").unwrap().host(), "127.0.0.1:8080");
    }

    #[test]
    fn covers_come_only_from_archive_org_and_deezer_over_https() {
        let allowed = |url: &str| is_allowed_cover_url(&reqwest::Url::parse(url).unwrap());
        assert!(allowed("https://archive.org/services/img/marea"));
        assert!(allowed("https://ia800608.us.archive.org/1/items/marea/cover.jpg"));
        assert!(allowed("https://cdn-images.dzcdn.net/images/cover/e2b36a9f/1000x1000-000000-80-0-0.jpg"));
        for url in [
            "https://dzcdn.net.evil.example/x.jpg",
            "http://cdn-images.dzcdn.net/images/cover/a/1.jpg",
            "http://archive.org/services/img/marea",
            "https://archive.org.evil.example/x.jpg",
            "https://evilarchive.org/x.jpg",
            "https://i.ytimg.com/vi/x/hq.jpg",
            "https://user@archive.org/x.jpg",
            "https://archive.org:8443/x.jpg",
        ] {
            assert!(!allowed(url), "{url}");
        }
    }

    #[test]
    fn storage_401_means_expired_url() {
        assert_eq!(check_storage_status(StatusCode::UNAUTHORIZED), Err(AppError::UrlExpired));
    }

    #[test]
    fn storage_403_means_expired_url() {
        assert_eq!(check_storage_status(StatusCode::FORBIDDEN), Err(AppError::UrlExpired));
    }

    #[test]
    fn storage_500_is_not_an_expired_url() {
        assert!(matches!(
            check_storage_status(StatusCode::INTERNAL_SERVER_ERROR),
            Err(AppError::Api { status: 500, .. })
        ));
    }

    #[test]
    fn content_range_gives_start_and_total() {
        assert_eq!(parse_content_range("bytes 100-199/1000"), Some((100, 1000)));
        assert_eq!(parse_content_range("bytes */1000"), None);
    }

    /// Minimal HTTP server with `Range` and a redirect, slow like an archive.org node: `/orig`
    /// sends to `/node`, which serves `SIZE` bytes with byte `i` equal to `i % 251`.
    ///
    /// Also acts as a backend for the tests on the renewal of signed URLs: `/v1/tracks/{id}/stream-url`
    /// and `/download-url` answer with a "signed" URL (`/signed/node` or `/signed/small`, with
    /// `sig=N` numbered in the order of issue, no real token) that the fake storage accepts or
    /// rejects with 403 according to the `Mode`.
    mod range_server {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        pub const SIZE: usize = 8 * 1024 * 1024;
        /// Size of the small file (`/signed/small`) of the download tests.
        pub const SMALL: usize = 4096;
        const CHUNK: usize = 16 * 1024;
        /// 16 KB every 10 ms: about 1.6 MB/s, the whole file in five seconds.
        const PAUSE: Duration = Duration::from_millis(10);

        /// How the fake storage treats the `/signed/...` URLs.
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub enum Mode {
            /// All the signed URLs are valid.
            Fresh,
            /// The first URL issued (`sig=1`) has already expired: 403 once, then everything goes.
            Expiring,
            /// Every signed URL has expired: 403 always.
            Expired,
        }

        pub fn byte(i: usize) -> u8 {
            (i % 251) as u8
        }

        /// Starts the server and returns the base and the list of requests (`path start`,
        /// without query).
        pub async fn start() -> (String, Arc<Mutex<Vec<String>>>) {
            start_with(Mode::Fresh).await
        }

        pub async fn start_with(mode: Mode) -> (String, Arc<Mutex<Vec<String>>>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let log = Arc::new(Mutex::new(Vec::new()));
            let issued = Arc::new(AtomicUsize::new(0));
            let (requests, origin) = (Arc::clone(&log), base.clone());
            tokio::spawn(async move {
                loop {
                    let (socket, _) = listener.accept().await.unwrap();
                    let (requests, issued, origin) =
                        (Arc::clone(&requests), Arc::clone(&issued), origin.clone());
                    tokio::spawn(serve(socket, requests, mode, issued, origin));
                }
            });
            (base, log)
        }

        async fn reply_with(socket: &mut tokio::net::TcpStream, status: &str, body: &[u8], kind: &str) {
            let head = format!(
                "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body).await;
        }

        async fn serve(
            mut socket: tokio::net::TcpStream,
            log: Arc<Mutex<Vec<String>>>,
            mode: Mode,
            issued: Arc<AtomicUsize>,
            origin: String,
        ) {
            let mut head = Vec::new();
            let mut one = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if socket.read(&mut one).await.unwrap_or(0) == 0 {
                    return;
                }
                head.push(one[0]);
            }
            let head = String::from_utf8_lossy(&head).to_lowercase();
            let target = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (path, query) = target.split_once('?').unwrap_or((&target, ""));
            let start: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="))
                .and_then(|range| range.trim().trim_end_matches('-').parse().ok())
                .unwrap_or(0);
            log.lock().unwrap().push(format!("{path} {start}"));
            // The "backend": issues a numbered signed URL (stream or download).
            if path.starts_with("/v1/tracks/")
                && (path.ends_with("/stream-url") || path.ends_with("/download-url"))
            {
                let sig = issued.fetch_add(1, Ordering::SeqCst) + 1;
                let json = if path.ends_with("/stream-url") {
                    format!(
                        r#"{{"kind":"stream","url":"{origin}/signed/node?sig={sig}","expires_at":"2099-01-01T00:00:00Z"}}"#
                    )
                } else {
                    format!(
                        r#"{{"kind":"download","url":"{origin}/signed/small?sig={sig}","expires_at":"2099-01-01T00:00:00Z","format":"flac","quality":"lossless"}}"#
                    )
                };
                reply_with(&mut socket, "200 OK", json.as_bytes(), "application/json").await;
                return;
            }
            // The "storage": rejects expired signed URLs as the real one does (403).
            let mut path = path;
            if let Some(rest) = path.strip_prefix("/signed") {
                let sig: usize = query.strip_prefix("sig=").and_then(|n| n.parse().ok()).unwrap_or(0);
                let expired = match mode {
                    Mode::Fresh => false,
                    Mode::Expiring => sig == 1,
                    Mode::Expired => true,
                };
                if expired {
                    reply_with(&mut socket, "403 Forbidden", b"", "text/plain").await;
                    return;
                }
                if rest == "/cut" {
                    // The small file, but the first request drops after 1024 bytes with the full length
                    // already announced; a `Range` request from a later point serves the rest.
                    let body: Vec<u8> = (0..SMALL).map(byte).collect();
                    let head = if start == 0 {
                        format!("HTTP/1.1 200 OK\r\ncontent-length: {SMALL}\r\nconnection: close\r\n\r\n")
                    } else {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {start}-{}/{SMALL}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            SMALL - 1,
                            SMALL - start
                        )
                    };
                    let _ = socket.write_all(head.as_bytes()).await;
                    let end = if start == 0 { 1024 } else { SMALL };
                    let _ = socket.write_all(&body[start.min(SMALL)..end]).await;
                    return;
                }
                if rest == "/small" {
                    let body: Vec<u8> = (0..SMALL).map(byte).collect();
                    reply_with(&mut socket, "200 OK", &body, "application/octet-stream").await;
                    return;
                }
                path = rest;
            }
            if path == "/orig" {
                let reply =
                    "HTTP/1.1 302 Found\r\nlocation: /node\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
                let _ = socket.write_all(reply.as_bytes()).await;
                return;
            }
            let reply = format!(
                "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {start}-{}/{SIZE}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                SIZE - 1,
                SIZE - start
            );
            if socket.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
            let mut at = start;
            while at < SIZE {
                let end = (at + CHUNK).min(SIZE);
                let body: Vec<u8> = (at..end).map(byte).collect();
                if socket.write_all(&body).await.is_err() {
                    return;
                }
                at = end;
                tokio::time::sleep(PAUSE).await;
            }
        }
    }

    fn signed(url: String) -> SignedUrl {
        SignedUrl {
            kind: "stream".into(),
            url,
            expires_at: String::new(),
            height: None,
            format: None,
            quality: None,
        }
    }

    #[tokio::test]
    async fn a_far_read_downloads_from_there_without_waiting_for_the_rest() {
        use std::io::{Read, Seek, SeekFrom};
        let (base, log) = range_server::start().await;
        let api = ApiClient::new(&base).unwrap();
        let stream = api.open_stream(&signed(format!("{base}/orig"))).await.unwrap();
        let target = 6 * 1024 * 1024;
        let started = std::time::Instant::now();
        let mut reader = stream.reader();
        let read = tokio::task::spawn_blocking(move || {
            reader.seek(SeekFrom::Start(target as u64)).unwrap();
            let mut out = [0u8; 4];
            reader.read_exact(&mut out).unwrap();
            out
        });
        let out = read.await.unwrap();
        stream.cancel();
        // In sequenza ci vorrebbero quasi quattro secondi.
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
        assert_eq!(out.to_vec(), (target..target + 4).map(range_server::byte).collect::<Vec<_>>());
        let jump = target - crate::player::stream::JUMP_LEAD;
        assert!(log.lock().unwrap().contains(&format!("/node {jump}")), "{:?}", log.lock().unwrap());
    }

    #[tokio::test]
    async fn a_second_listen_goes_straight_to_the_resolved_node() {
        let (base, log) = range_server::start().await;
        let api = ApiClient::new(&base).unwrap();
        let url = signed(format!("{base}/orig"));
        api.open_stream(&url).await.unwrap().cancel();
        api.open_stream(&url).await.unwrap().cancel();
        let requests = log.lock().unwrap().clone();
        assert_eq!(requests, ["/orig 0", "/node 0", "/node 0"]);
    }

    #[tokio::test]
    async fn a_held_stream_stops_on_the_head_and_resumes_on_release() {
        let (base, _) = range_server::start().await;
        let api = ApiClient::new(&base).unwrap();
        let hold = 256 * 1024;
        let stream = api.open_stream_held(&signed(format!("{base}/orig")), Some(hold)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        let held = stream.head(usize::MAX).len();
        assert!(held >= hold && held < 4 * hold, "held at {held}");
        assert!(stream.is_held());
        stream.release();
        tokio::time::timeout(Duration::from_secs(15), stream.ready(usize::MAX)).await.unwrap();
        assert_eq!(stream.snapshot().len(), range_server::SIZE);
    }

    #[tokio::test]
    async fn a_jump_fills_the_gap_and_the_buffer_completes() {
        use std::io::{Read, Seek, SeekFrom};
        let (base, _) = range_server::start().await;
        let api = ApiClient::new(&base).unwrap();
        let stream = api.open_stream(&signed(format!("{base}/orig"))).await.unwrap();
        let mut reader = stream.reader();
        tokio::task::spawn_blocking(move || {
            reader.seek(SeekFrom::Start(7 * 1024 * 1024)).unwrap();
            reader.read_exact(&mut [0u8; 1]).unwrap();
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(15), stream.ready(usize::MAX)).await.unwrap();
        let all = stream.snapshot();
        assert_eq!(all.len(), range_server::SIZE);
        assert!(all.iter().enumerate().all(|(i, &b)| b == range_server::byte(i)));
    }

    // ---------- Renewal of signed URLs and downloads, end-to-end on a local server ----------

    /// Requests made to the fake server at the path that ends with `suffix` (one item per request).
    fn hits(log: &Mutex<Vec<String>>, suffix: &str) -> usize {
        log.lock()
            .unwrap()
            .iter()
            .filter(|line| line.split(' ').next().is_some_and(|p| p.ends_with(suffix)))
            .count()
    }

    /// Empty and unique folder for a download test; it removes it at the end.
    struct ScratchDir(std::path::PathBuf);

    impl ScratchDir {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("psytify-api-test-{}", uuid::Uuid::new_v4())))
        }

        fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .map(|dir| dir.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                .unwrap_or_default();
            names.sort();
            names
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn an_expired_stream_url_is_refreshed_once() {
        let (base, log) = range_server::start_with(range_server::Mode::Expiring).await;
        let api = ApiClient::new(&base).unwrap();
        let id = uuid::Uuid::from_u128(1);
        let stream = crate::api::stream_audio(api, id, crate::api::Quality::Lossless).await.unwrap();
        stream.cancel();
        // A new URL at each attempt: the first storage rejects it (403), the second serves.
        assert_eq!(hits(&log, "/stream-url"), 2);
        assert_eq!(hits(&log, "/signed/node"), 2);
        let order: Vec<String> = log
            .lock()
            .unwrap()
            .iter()
            .map(|line| line.rsplit_once('/').map_or(line.as_str(), |(_, tail)| tail).to_owned())
            .collect();
        assert_eq!(order, ["stream-url 0", "node 0", "stream-url 0", "node 0"]);
    }

    #[tokio::test]
    async fn a_second_expiry_is_reported() {
        let (base, log) = range_server::start_with(range_server::Mode::Expired).await;
        let api = ApiClient::new(&base).unwrap();
        let id = uuid::Uuid::from_u128(1);
        let result = crate::api::stream_audio(api, id, crate::api::Quality::Lossless).await;
        assert!(matches!(result, Err(AppError::UrlExpired)));
        // It is tried only once more; then the error reaches the caller, without other attempts.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(hits(&log, "/stream-url"), 2);
        assert_eq!(hits(&log, "/signed/node"), 2);
    }

    #[tokio::test]
    async fn a_download_refreshes_once_and_never_overwrites() {
        let (base, log) = range_server::start_with(range_server::Mode::Expiring).await;
        let api = ApiClient::new(&base).unwrap();
        let track = crate::api::models::fixtures::track(1);
        let dir = ScratchDir::new();
        std::fs::create_dir_all(&dir.0).unwrap();
        // The file with the name the download would have (title + format given by the server) already exists.
        std::fs::write(dir.0.join("Brano 1.flac"), b"mio").unwrap();

        let progress = Arc::new(Progress::default());
        let done = crate::api::download_track(
            api,
            track,
            dir.0.clone(),
            crate::api::DownloadQuality::Lossless,
            Arc::clone(&progress),
        )
        .await
        .unwrap();

        assert_eq!(hits(&log, "/download-url"), 2);
        assert_eq!(hits(&log, "/signed/small"), 2);
        assert_eq!(done.path, dir.0.join("Brano 1 (2).flac"));
        assert_eq!(done.delivered, Some(crate::api::DownloadQuality::Lossless));
        assert!(!done.is_lower_than_asked());
        assert_eq!(std::fs::read(dir.0.join("Brano 1.flac")).unwrap(), b"mio");
        let expected: Vec<u8> = (0..range_server::SMALL).map(range_server::byte).collect();
        assert_eq!(std::fs::read(&done.path).unwrap(), expected);
        // Weight and SHA-256 are those of the written file, and the progress counted all the bytes.
        assert_eq!(done.size, range_server::SMALL as u64);
        assert_eq!(done.sha256, sha256_hex(&expected));
        assert_eq!(done.format, "flac");
        assert_eq!(
            (progress.received(), progress.total()),
            (range_server::SMALL as u64, Some(range_server::SMALL as u64))
        );
        // Nessun `.part` rimasto.
        assert_eq!(dir.files(), ["Brano 1 (2).flac", "Brano 1.flac"]);
    }

    #[tokio::test]
    async fn a_download_with_a_second_expiry_leaves_the_folder_untouched() {
        let (base, log) = range_server::start_with(range_server::Mode::Expired).await;
        let api = ApiClient::new(&base).unwrap();
        let dir = ScratchDir::new();
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join("Brano 1.flac"), b"mio").unwrap();

        let result = crate::api::download_track(
            api,
            crate::api::models::fixtures::track(1),
            dir.0.clone(),
            crate::api::DownloadQuality::Lossless,
            Arc::default(),
        )
        .await;

        assert!(matches!(result, Err(AppError::UrlExpired)));
        assert_eq!(hits(&log, "/download-url"), 2);
        assert_eq!(hits(&log, "/signed/small"), 2);
        assert_eq!(std::fs::read(dir.0.join("Brano 1.flac")).unwrap(), b"mio");
        assert_eq!(dir.files(), ["Brano 1.flac"]);
    }

    /// BACKLOG 3: the temporary file has a unique name and is not `<title>.part`. A user's file with that
    /// name (the old temporary of "Brano 1 (2).flac") is not truncated, and once the download ends it does not remain
    /// nessun temporaneo.
    #[tokio::test]
    async fn a_download_never_touches_an_unrelated_part_file_and_leaves_no_temp() {
        let (base, _) = range_server::start().await;
        let api = ApiClient::new(&base).unwrap();
        let dir = ScratchDir::new();
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join("Brano 1.flac"), b"mio").unwrap();
        std::fs::write(dir.0.join("Brano 1 (2).part"), b"altro").unwrap();

        let done = crate::api::download_track(
            api,
            crate::api::models::fixtures::track(1),
            dir.0.clone(),
            crate::api::DownloadQuality::Lossless,
            Arc::default(),
        )
        .await
        .unwrap();

        assert_eq!(done.path, dir.0.join("Brano 1 (2).flac"));
        assert_eq!(std::fs::read(dir.0.join("Brano 1 (2).part")).unwrap(), b"altro");
        assert_eq!(dir.files(), ["Brano 1 (2).flac", "Brano 1 (2).part", "Brano 1.flac"]);
    }

    /// An interrupted download (the future dropped, as on a cancel) removes its temporary file.
    #[tokio::test]
    async fn a_dropped_download_removes_its_temp_file() {
        let (base, _) = range_server::start().await;
        let api = ApiClient::new(&base).unwrap();
        let dir = ScratchDir::new();
        std::fs::create_dir_all(&dir.0).unwrap();
        // `/node` is the slow 8 MiB file: after 200 ms it is halfway.
        let (url, progress) = (format!("{base}/signed/node?sig=9"), Progress::default());
        let download = api.fetch_into(&url, &dir.0, &progress);
        assert!(tokio::time::timeout(Duration::from_millis(200), download).await.is_err());
        assert_eq!(dir.files(), Vec::<String>::new());
    }

    /// The connection falls halfway: the download restarts from where it had reached and the file is whole, with
    /// the right SHA-256.
    #[tokio::test]
    async fn a_dropped_connection_resumes_from_where_it_was() {
        let (base, log) = range_server::start().await;
        let api = ApiClient::new(&base).unwrap();
        let dir = ScratchDir::new();
        std::fs::create_dir_all(&dir.0).unwrap();
        let progress = Progress::default();
        let fetched = api.fetch_into(&format!("{base}/signed/cut?sig=1"), &dir.0, &progress).await.unwrap();
        let expected: Vec<u8> = (0..range_server::SMALL).map(range_server::byte).collect();
        assert_eq!(fetched.size, range_server::SMALL as u64);
        assert_eq!(fetched.sha256, sha256_hex(&expected));
        assert_eq!(std::fs::read(fetched.temp.path()).unwrap(), expected);
        let starts: Vec<String> = log.lock().unwrap().clone();
        assert_eq!(
            starts,
            ["/signed/cut 0", "/signed/cut 1024"],
            "la seconda richiesta parte dal punto di arrivo"
        );
        assert_eq!(progress.received(), range_server::SMALL as u64);
    }

    /// A release with a name that is not a simple file writes nothing and makes no requests.
    #[tokio::test]
    async fn a_release_asset_with_a_path_in_its_name_is_refused() {
        let (base, log) = range_server::start().await;
        let api = ApiClient::new(&base).unwrap();
        let dir = ScratchDir::new();
        let asset = update::Asset { name: "../../evil.tar.gz".into(), size: 1 };
        let result = api.download_release(&asset, &dir.0, &Progress::default()).await;
        assert_eq!(result, Err(AppError::InvalidResponse));
        assert!(log.lock().unwrap().is_empty());
        assert!(!dir.0.exists());
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        crate::offline::hex(&Sha256::digest(bytes))
    }
}
