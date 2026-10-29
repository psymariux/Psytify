//! Client HTTP verso il backend Psytify e verso gli URL firmati dello storage.
//!
//! Ogni richiesta ha un timeout. Gli URL firmati non vengono mai registrati né restituiti dentro
//! un errore: `AppError` conserva solo status e codice.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::StatusCode;

use super::update;
use serde::de::DeserializeOwned;
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

/// Timeout complessivo delle chiamate al catalogo.
pub const API_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout complessivo per scaricare un brano tutto in memoria (test sul server vivo).
#[cfg(test)]
pub const AUDIO_TIMEOUT: Duration = Duration::from_secs(120);
/// Un download lungo non ha un timeout totale, ma si interrompe se non arrivano dati.
pub const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// L'importazione legge i metadati da Archive: può richiedere qualche secondo in più.
const IMPORT_TIMEOUT: Duration = Duration::from_secs(30);
/// Ricerca sulle fonti esterne: il server interroga tre fonti in parallelo.
const SOURCE_SEARCH_TIMEOUT: Duration = Duration::from_secs(45);
/// L'importazione da una fonte esterna scarica l'audio sul server (il server si ferma a 120 secondi).
const SOURCE_IMPORT_TIMEOUT: Duration = Duration::from_secs(130);
/// Catalogo ufficiale: il server interroga Deezer (più pagine per la discografia di un artista).
const META_TIMEOUT: Duration = Duration::from_secs(25);
/// Il primo `video-url` di un brano scarica il videoclip sul server (che si ferma a 120 secondi).
#[cfg(feature = "video")]
const VIDEO_TIMEOUT: Duration = Duration::from_secs(130);
/// La preparazione di un videoclip risponde subito: il download resta sul server.
const PREPARE_TIMEOUT: Duration = Duration::from_secs(15);
/// Copertine: timeout e dimensione massima di un'immagine.
const COVER_TIMEOUT: Duration = Duration::from_secs(20);
/// Il backend attende LRCLIB (fino a 10 s per ricerca) prima di rispondere.
const LYRICS_TIMEOUT: Duration = Duration::from_secs(25);
pub const MAX_COVER_BYTES: usize = 4 * 1024 * 1024;
/// Redirect seguiti al massimo per una copertina (archive.org manda a un nodo di archiviazione).
const MAX_COVER_REDIRECTS: usize = 5;
/// Oltre questa dimensione un brano non viene caricato in memoria per l'ascolto.
pub const MAX_STREAM_BYTES: u64 = 600 * 1024 * 1024;
/// Tentativi di una richiesta `Range` dopo un salto, prima di dare il brano per perso.
const RANGE_ATTEMPTS: usize = 2;
/// URL risolti ricordati al massimo: oltre si ricomincia da capo.
const MAX_RESOLVED: usize = 256;
/// Ogni quanto un download fermato sulla testata controlla se può ripartire.
const HOLD_POLL: Duration = Duration::from_millis(25);
/// Quante volte un download caduto a metà riparte da dove era arrivato.
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

/// Risposta di `download-url`: l'URL pronto, oppure la versione chiesta è ancora in preparazione sul
/// server (la ricodifica) e va richiesta di nuovo fra poco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadReply {
    Ready(SignedUrl),
    Preparing(Duration),
}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    download: reqwest::Client,
    /// Solo copertine, chieste al backend. Il controllo sull'host dell'immagine (`is_allowed_cover_url`)
    /// resta: il backend lo rifà, ma il client non chiede mai altro.
    covers: reqwest::Client,
    base: Arc<str>,
    /// URL d'ascolto → URL finale dopo i redirect, per la sessione. archive.org manda ogni
    /// richiesta a un nodo di archiviazione e il redirect costa circa 2,7 secondi: riascoltare un
    /// brano o saltarci dentro va subito al nodo. Solo in memoria, mai nei log.
    resolved: Arc<Mutex<HashMap<String, reqwest::Url>>>,
}

/// Un URL di copertina si scarica solo in https da `archive.org` o da un suo sottodominio
/// (`ia800608.us.archive.org`), o dal CDN delle immagini di Deezer (`cdn-images.dzcdn.net`), da
/// cui il backend prende le copertine ufficiali dei brani di YouTube e le foto degli artisti. Sono
/// le uniche eccezioni ammesse agli URL risolti dal backend, e solo per le immagini.
pub fn is_allowed_cover_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|host| {
            host == "archive.org"
                || host.ends_with(".archive.org")
                || host.ends_with(".dzcdn.net")
                // Gli avatar dei profili.
                || host == "img.clerk.com"
        })
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient").field("base", &self.base).finish_non_exhaustive()
    }
}

impl ApiClient {
    pub fn new(base_url: &str) -> Result<Self, AppError> {
        let base = base_url.trim().trim_end_matches('/');
        let parsed = reqwest::Url::parse(base).map_err(|_| AppError::InvalidBaseUrl(base.into()))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(AppError::InvalidBaseUrl(base.into()));
        }
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

    /// Host e porta del backend, da mostrare nell'interfaccia.
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

    /// Come `get_json`, con un tempo massimo proprio: il backend può attendere un servizio esterno
    /// (LRCLIB) più a lungo di una richiesta del catalogo.
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

    /// Ricerca nel catalogo: risultato principale, brani, artisti e album.
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

    /// Cerca nel catalogo ufficiale: brani, artisti, album e playlist.
    pub async fn meta_search(&self, q: &str) -> Result<MetaSearch, AppError> {
        let dto: MetaSearchDto =
            self.get_json_within("/v1/meta/search", &[("q", q.to_owned())], META_TIMEOUT).await?;
        Ok(dto.into())
    }

    /// L'artista con questo nome: brani popolari e tutta la discografia.
    pub async fn meta_artist_named(&self, name: &str) -> Result<MetaArtistPage, AppError> {
        let dto: MetaArtistPageDto =
            self.get_json_within("/v1/meta/artists", &[("name", name.to_owned())], META_TIMEOUT).await?;
        Ok(dto.into())
    }

    /// Porta un artista del catalogo nel server e restituisce il suo identificatore (UUID).
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

    /// Album o playlist del catalogo (`album/123`, `playlist/456`) con i loro brani.
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

    /// I brani di una playlist esterna. L'identificatore è quello ricevuto dal server.
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

    /// Porta un risultato della ricerca esterna nel catalogo e restituisce il brano. `source` è uno
    /// di `youtube`, `soundcloud`, `bandcamp`; l'identificatore è quello ricevuto dal server.
    /// Con `video` (solo per il catalogo ufficiale) il server scarica il videoclip insieme all'audio.
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

    /// Byte di una copertina, dal backend (`/v1/cover`): il client non apre connessioni verso Deezer
    /// o Internet Archive. L'indirizzo originale si controlla comunque prima di chiederlo, al massimo
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

    /// Un solo brano del catalogo: la jam lo usa per i brani della coda condivisa che non ha.
    pub async fn track(&self, id: Uuid) -> Result<Track, AppError> {
        self.get_json(&format!("/v1/catalog/tracks/{id}"), &[]).await
    }

    pub async fn stream_url(&self, id: Uuid, quality: Quality) -> Result<SignedUrl, AppError> {
        let signed = self
            .get_json(&format!("/v1/tracks/{id}/stream-url"), &[("quality", quality.as_str().to_owned())])
            .await?;
        Ok(self.absolute(signed))
    }

    /// Un brano o un videoclip appena importati li serve il backend stesso, dal suo disco: l'URL è
    /// relativo alla sua origine.
    fn absolute(&self, mut signed: SignedUrl) -> SignedUrl {
        if signed.url.starts_with("/v1/") {
            signed.url = format!("{}{}", self.base, signed.url);
        }
        signed
    }

    /// Testi del brano. `found: false` se non ci sono; `503` se il backend non raggiunge LRCLIB.
    pub async fn lyrics(&self, id: Uuid) -> Result<Lyrics, AppError> {
        self.get_json_within(&format!("/v1/tracks/{id}/lyrics"), &[], LYRICS_TIMEOUT).await
    }

    /// URL firmato del videoclip; `404` se il brano non ne ha.
    #[cfg(feature = "video")]
    pub async fn video_url(&self, id: Uuid) -> Result<SignedUrl, AppError> {
        let signed = self.get_json_within(&format!("/v1/tracks/{id}/video-url"), &[], VIDEO_TIMEOUT).await?;
        Ok(self.absolute(signed))
    }

    /// Chiede al server di preparare il videoclip di un brano che suonerà presto: risponde subito.
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

    /// Una richiesta con il login (profili, amici, attività). `None` per le risposte senza corpo
    /// (`204`); `401` diventa `AuthExpired`, i messaggi del server restano quelli di `api_error`.
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

    /// L'utente del token di Clerk. `401` vuol dire token scaduto o revocato.
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
        // 202: il server sta ricodificando la versione chiesta; `Retry-After` dice quando riprovare.
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

    /// Apre l'ascolto progressivo: controlla subito lo stato (401 e 403 vogliono dire URL scaduto),
    /// poi un task continua a scaricare nel buffer. Torna appena ci sono i byte per partire.
    ///
    /// Si chiede il file con `Range: bytes=0-`: se il server risponde 206, il buffer conosce la
    /// lunghezza e un seek lontano fa ripartire il download da quel punto (vedi `player::stream`).
    #[cfg(test)]
    pub async fn open_stream(&self, url: &SignedUrl) -> Result<StreamBuffer, AppError> {
        self.open_stream_held(url, None).await
    }

    /// Come [`Self::open_stream`], ma con `hold` il download si ferma dopo quei byte iniziali e
    /// riparte con `StreamBuffer::release`: serve a tenere pronta la testata di un brano che
    /// forse si ascolterà, senza scaricarlo tutto.
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

    /// Prima risposta dello stream e URL finale da usare per i salti. Prova prima il nodo già
    /// risolto; se non risponde, torna all'URL originale.
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

    /// Cerca su Internet Archive la musica che non è nel catalogo.
    pub async fn discover(&self, q: &str) -> Result<DiscoverPage, AppError> {
        self.get_json("/v1/discover", &[("q", q.to_owned())]).await
    }

    /// Una pagina (da 1) degli album con licenza libera di una libreria di Archive.
    pub async fn library(&self, identifier: &str, page: u32) -> Result<LibraryPage, AppError> {
        if !is_archive_identifier(identifier) {
            return Err(AppError::InvalidResponse);
        }
        self.get_json(&format!("/v1/libraries/{identifier}"), &[("page", page.to_string())]).await
    }

    /// Porta un item di Archive nel catalogo e restituisce i suoi brani, pronti da suonare.
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

    /// Scarica un URL firmato su disco: prima in `<dest>.part`, poi rinomina.
    /// L'ultima release dell'app, come la legge il backend.
    pub async fn latest_release(&self) -> Result<update::Release, AppError> {
        self.get_json("/v1/app/release", &[]).await
    }

    /// Scarica un file dell'ultima release dal backend, nel file `dest`.
    pub async fn download_release(&self, asset: &update::Asset, dest: &Path) -> Result<u64, AppError> {
        self.fetch_to(&format!("{}/v1/app/release/assets/{}", self.base, asset.name), dest).await
    }

    pub async fn download_to(&self, url: &SignedUrl, dest: &Path) -> Result<u64, AppError> {
        self.fetch_to(&url.url, dest).await
    }

    /// Scarica `url` a blocchi nel file `dest`: prima in `dest.part`, poi lo rinomina.
    async fn fetch_to(&self, url: &str, dest: &Path) -> Result<u64, AppError> {
        let mut response = self.download.get(url).send().await.map_err(|e| AppError::from_http(&e))?;
        check_storage_status(response.status())?;

        let partial = dest.with_extension("part");
        let io_error = |_| AppError::Io(dest.display().to_string());
        let mut file = tokio::fs::File::create(&partial).await.map_err(io_error)?;
        let mut written = 0u64;
        let result = async {
            while let Some(chunk) = response.chunk().await.map_err(|e| AppError::from_http(&e))? {
                file.write_all(&chunk).await.map_err(io_error)?;
                written += chunk.len() as u64;
            }
            file.flush().await.map_err(io_error)?;
            Ok::<_, AppError>(())
        }
        .await;
        drop(file);
        if let Err(err) = result {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(err);
        }
        tokio::fs::rename(&partial, dest).await.map_err(io_error)?;
        Ok(written)
    }
}

async fn range_request(
    client: &reqwest::Client,
    url: reqwest::Url,
    from: usize,
) -> reqwest::Result<reqwest::Response> {
    client.get(url).header(reqwest::header::RANGE, format!("bytes={from}-")).send().await
}

/// Inizio e lunghezza totale da `Content-Range: bytes 100-199/1000`.
fn content_range(response: &reqwest::Response) -> Option<(u64, u64)> {
    let value = response.headers().get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    parse_content_range(value)
}

fn parse_content_range(value: &str) -> Option<(u64, u64)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// Scrive nel buffer la risposta in corso e, quando il buffer lo chiede, ne apre un'altra da un
/// altro punto del file (un seek lontano, poi i buchi rimasti).
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
    // Una risposta caduta a metà (il server chiude una connessione ferma da un po') si riapre dal
    // punto in cui era arrivata, poche volte.
    let mut reopened = 0;
    loop {
        let Some(at) = offset else {
            writer.fail();
            return;
        };
        // Testata pronta e nessun ascolto in corso: si aspetta, senza consumare banda.
        while writer.is_held() && !writer.is_cancelled() {
            tokio::time::sleep(HOLD_POLL).await;
        }
        if writer.is_cancelled() {
            return;
        }
        let arrival = tokio::select! {
            chunk = response.chunk() => Some(chunk),
            () = writer.jump_requested() => None,
        };
        let next = match arrival {
            // Un lettore chiede un salto: un blocco vuoto fa dire al buffer dove andare.
            None => match writer.write_at(at, &[]) {
                Next::Continue(_) => continue,
                other => other,
            },
            Some(Ok(Some(chunk))) => {
                received += chunk.len() as u64;
                // Senza lunghezza annunciata il limite si controlla qui; con i salti un byte può
                // arrivare due volte, quindi il conto ha margine.
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

/// Identificatore di Archive: viene dal server ma finisce in un percorso, quindi si ricontrolla.
fn is_archive_identifier(identifier: &str) -> bool {
    identifier.len() <= 100
        && identifier.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && identifier.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Messaggio in italiano per ogni codice stabile di docs/BACKEND.md.
/// Un identificatore delle fonti esterne sta in un solo segmento di percorso: lettere, cifre, `-`
/// e `_`, al massimo 200 caratteri.
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

    /// Server HTTP minimo con `Range` e un redirect, lento come un nodo di archive.org: `/orig`
    /// manda a `/node`, che serve `SIZE` byte con il byte `i` uguale a `i % 251`.
    mod range_server {
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        pub const SIZE: usize = 8 * 1024 * 1024;
        const CHUNK: usize = 16 * 1024;
        /// 16 KB ogni 10 ms: circa 1,6 MB/s, tutto il file in cinque secondi.
        const PAUSE: Duration = Duration::from_millis(10);

        pub fn byte(i: usize) -> u8 {
            (i % 251) as u8
        }

        /// Avvia il server e restituisce la base e l'elenco delle richieste (`percorso inizio`).
        pub async fn start() -> (String, Arc<Mutex<Vec<String>>>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let log = Arc::new(Mutex::new(Vec::new()));
            let requests = Arc::clone(&log);
            tokio::spawn(async move {
                loop {
                    let (socket, _) = listener.accept().await.unwrap();
                    tokio::spawn(serve(socket, Arc::clone(&requests)));
                }
            });
            (base, log)
        }

        async fn serve(mut socket: tokio::net::TcpStream, log: Arc<Mutex<Vec<String>>>) {
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if socket.read(&mut byte).await.unwrap_or(0) == 0 {
                    return;
                }
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).to_lowercase();
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let start: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="))
                .and_then(|range| range.trim().trim_end_matches('-').parse().ok())
                .unwrap_or(0);
            log.lock().unwrap().push(format!("{path} {start}"));
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
                let body: Vec<u8> = (at..end).map(super::range_server::byte).collect();
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
}
