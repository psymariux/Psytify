//! Accesso al backend: client HTTP, modelli e flussi che combinano più richieste.

pub mod client;
pub mod models;
pub mod social;
pub mod update;

use std::future::Future;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

use uuid::Uuid;

pub use client::{ApiClient, DownloadReply, Sort, TrackQuery};
pub use models::{
    Album, AlbumSummary, ArtistPage, ArtistSummary, DiscoverImport, DiscoverItem, DiscoverLibrary,
    DiscoverPage, DownloadQuality, LibraryPage, Lyrics, Me, MetaArtist, MetaArtistPage, MetaSearch, Page,
    Quality, SearchResults, SignedUrl, SourceItem, SourceKind, SourcePlaylist, SourcePlaylistPage, TopKind,
    Track,
};

use crate::error::AppError;
use crate::player::decoder;
use crate::player::stream::{StreamBuffer, WARM_BYTES};

/// Nome senza spazi né punteggiatura, in minuscolo, per confrontare titoli e artisti.
fn compact(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Fra i brani del catalogo, quello che è lo stesso del brano salvato: stesso artista e titolo, e
/// la durata più vicina (entro 15 secondi, se la conosce).
pub fn best_match<'a>(track: &Track, found: &'a [SourceItem]) -> Option<&'a SourceItem> {
    let (artist, title) = (compact(&track.artist.name), compact(&track.title));
    found
        .iter()
        .filter(|item| compact(&item.artist) == artist && compact(&item.title) == title)
        .filter(|item| {
            item.duration_ms
                .is_none_or(|ms| ms.abs_diff(track.duration_ms) <= 15_000 || track.duration_ms == 0)
        })
        .min_by_key(|item| item.duration_ms.map_or(0, |ms| ms.abs_diff(track.duration_ms)))
}

/// Un brano salvato che il server non conosce più (database ricreato): lo ritrova nel catalogo
/// ufficiale e lo fa importare di nuovo. Restituisce il brano nuovo, con il suo identificatore.
pub async fn heal_track(api: ApiClient, track: Track) -> Result<Track, AppError> {
    let query = format!("{} {}", track.artist.name, track.title);
    let found = api.meta_search(&query).await?;
    let item = best_match(&track, &found.tracks).ok_or(AppError::InvalidResponse)?;
    let imported = api.import_source(item.source, &item.id, false).await?;
    imported.tracks.into_iter().next().ok_or(AppError::InvalidResponse)
}

/// Richiede un URL firmato e lo usa. Se lo storage risponde 401 o 403 l'URL è scaduto: ne
/// richiede uno nuovo e riprova una sola volta.
pub async fn with_fresh_url<T, U, UF, F, FF>(mut request_url: U, mut use_url: F) -> Result<T, AppError>
where
    U: FnMut() -> UF,
    UF: Future<Output = Result<SignedUrl, AppError>>,
    F: FnMut(SignedUrl) -> FF,
    FF: Future<Output = Result<T, AppError>>,
{
    let url = request_url().await?;
    match use_url(url).await {
        Err(AppError::UrlExpired) => {
            let fresh = request_url().await?;
            use_url(fresh).await
        }
        other => other,
    }
}

/// Apre l'ascolto progressivo di un brano nella qualità scelta: il buffer ha già i primi byte e
/// continua a riempirsi. Un MP4 (M4A) con l'indice in fondo va letto dalla fine: se il server
/// non ha detto la lunghezza, si aspetta il file intero.
pub async fn stream_audio(
    api: ApiClient,
    track_id: Uuid,
    quality: Quality,
) -> Result<StreamBuffer, AppError> {
    open_audio(api, track_id, quality, None).await
}

/// Apre un brano che forse si ascolterà: scarica solo la testata (`WARM_BYTES`) e si ferma. Se
/// poi parte, `StreamBuffer::release` (o il primo byte letto oltre la testata) lo fa continuare.
pub async fn warm_audio(api: ApiClient, track_id: Uuid, quality: Quality) -> Result<StreamBuffer, AppError> {
    open_audio(api, track_id, quality, Some(WARM_BYTES)).await
}

async fn open_audio(
    api: ApiClient,
    track_id: Uuid,
    quality: Quality,
    hold: Option<usize>,
) -> Result<StreamBuffer, AppError> {
    let stream = with_fresh_url(
        || {
            let api = api.clone();
            async move { api.stream_url(track_id, quality).await }
        },
        |url| {
            let api = api.clone();
            async move { api.open_stream_held(&url, hold).await }
        },
    )
    .await?;
    if stream.total().is_none()
        && stream.ready(decoder::SNIFF_BYTES).await
        && decoder::is_mp4(&stream.head(decoder::SNIFF_BYTES))
    {
        // Un M4A senza lunghezza si decodifica solo a file intero: niente testata sola.
        stream.release();
        // `ready(usize::MAX)` aspetta la fine del download, o `false` se è fallito.
        if !stream.ready(usize::MAX).await {
            return Err(AppError::AudioDecode);
        }
    }
    Ok(stream)
}

/// Byte audio di un brano nella qualità scelta, tutti in memoria (test sul server vivo).
#[cfg(test)]
pub async fn load_audio(api: ApiClient, track_id: Uuid, quality: Quality) -> Result<Arc<[u8]>, AppError> {
    let bytes = with_fresh_url(
        || {
            let api = api.clone();
            async move { api.stream_url(track_id, quality).await }
        },
        |url| {
            let api = api.clone();
            async move { api.fetch_audio(&url).await }
        },
    )
    .await?;
    Ok(Arc::from(bytes))
}

/// Quante volte si richiede una versione in preparazione prima di arrendersi: il server ricodifica
/// al massimo per tre minuti.
const PREPARE_ATTEMPTS: usize = 90;

/// Un brano scaricato: dove è finito e che qualità ha davvero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    pub path: PathBuf,
    /// La qualità che il server ha dato; `None` se non l'ha detta (un server più vecchio).
    pub delivered: Option<DownloadQuality>,
    /// La qualità chiesta.
    pub asked: DownloadQuality,
}

impl Downloaded {
    /// Il brano non ha la qualità chiesta (un file con perdita non si migliora): è arrivata la più vicina.
    pub fn is_lower_than_asked(&self) -> bool {
        self.delivered.is_some_and(|got| got != self.asked)
    }
}

/// Chiede l'URL di download nella qualità scelta, aspettando che il server finisca di preparare la
/// versione se serve una ricodifica.
async fn ready_download_url(
    api: &ApiClient,
    id: Uuid,
    quality: DownloadQuality,
) -> Result<SignedUrl, AppError> {
    for _ in 0..PREPARE_ATTEMPTS {
        match api.download_url(id, quality).await? {
            DownloadReply::Ready(url) => return Ok(url),
            DownloadReply::Preparing(wait) => tokio::time::sleep(wait).await,
        }
    }
    Err(AppError::Timeout)
}

/// Scarica un brano nella cartella indicata, nella qualità chiesta, e restituisce il percorso del
/// file. L'estensione è quella del file che il server dà davvero (una versione ricodificata è m4a).
pub async fn download_track(
    api: ApiClient,
    track: Track,
    dir: PathBuf,
    quality: DownloadQuality,
) -> Result<Downloaded, AppError> {
    tokio::fs::create_dir_all(&dir).await.map_err(|_| AppError::Io(dir.display().to_string()))?;
    let id = track.id;
    with_fresh_url(
        || {
            let api = api.clone();
            async move { ready_download_url(&api, id, quality).await }
        },
        |url| {
            let api = api.clone();
            let (dir, track) = (dir.clone(), track.clone());
            async move {
                let extension =
                    url.format.as_deref().filter(|f| is_known_extension(f)).unwrap_or(track.extension());
                let dest = unique_path(&dir, &download_filename(&track.title, extension)).await;
                api.download_to(&url, &dest).await?;
                let delivered = url.quality.as_deref().and_then(DownloadQuality::parse);
                Ok(Downloaded { path: dest, delivered, asked: quality })
            }
        },
    )
    .await
}

fn is_known_extension(extension: &str) -> bool {
    matches!(extension, "flac" | "mp3" | "ogg" | "wav" | "m4a")
}

/// Stessa regola del server: lettere e cifre ASCII, spazi, trattini e underscore, al massimo
/// 100 caratteri; se resta vuoto `track`.
pub fn download_filename(title: &str, ext: &str) -> String {
    let filtered: String = title
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .collect();
    let collapsed = filtered.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem: String = collapsed.chars().take(100).collect();
    let stem = stem.trim();
    format!("{}.{ext}", if stem.is_empty() { "track" } else { stem })
}

/// Non sovrascrive mai un file esistente: aggiunge " (2)", " (3)", ...
async fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
        return candidate;
    }
    let (stem, ext) = filename.rsplit_once('.').unwrap_or((filename, ""));
    for n in 2..1000 {
        let path = dir.join(format!("{stem} ({n}).{ext}"));
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return path;
        }
    }
    dir.join(format!("{stem} ({}).{ext}", Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn item(id: &str, title: &str, artist: &str, duration_ms: u64) -> SourceItem {
        SourceItem {
            source: SourceKind::Deezer,
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            duration_ms: Some(duration_ms),
            cover_url: None,
        }
    }

    #[test]
    fn a_lost_track_is_matched_by_artist_title_and_duration() {
        let mut saved = models::fixtures::track(1);
        saved.title = "Paper Cuts".into();
        saved.artist.name = "Linkin Park".into();
        saved.duration_ms = 187_000;
        let found = [
            item("1", "Paper Cuts (Live)", "Linkin Park", 190_000),
            item("2", "Paper cuts", "Linkin  Park", 250_000),
            item("3", "Paper Cuts", "Linkin Park", 188_000),
            item("4", "Paper Cuts", "Someone Else", 187_000),
        ];
        assert_eq!(best_match(&saved, &found).map(|i| i.id.as_str()), Some("3"));
        assert!(best_match(&saved, &found[..2]).is_none());
    }

    fn signed(n: usize) -> SignedUrl {
        SignedUrl {
            kind: "stream".into(),
            url: format!("https://storage/{n}"),
            expires_at: String::new(),
            height: None,
            format: None,
            quality: None,
        }
    }

    /// Simula backend e storage: conta le richieste e risponde con gli esiti indicati, in ordine.
    async fn run(outcomes: Vec<Result<u8, AppError>>) -> (Result<u8, AppError>, usize, usize) {
        let url_requests = AtomicUsize::new(0);
        let fetches = AtomicUsize::new(0);
        let outcomes = std::sync::Mutex::new(outcomes.into_iter());
        let result = with_fresh_url(
            || {
                let n = url_requests.fetch_add(1, Ordering::SeqCst);
                async move { Ok(signed(n)) }
            },
            |_url| {
                fetches.fetch_add(1, Ordering::SeqCst);
                let next = outcomes.lock().unwrap().next().unwrap();
                async move { next }
            },
        )
        .await;
        (result, url_requests.into_inner(), fetches.into_inner())
    }

    #[tokio::test]
    async fn valid_url_is_used_once() {
        assert_eq!(run(vec![Ok(7)]).await, (Ok(7), 1, 1));
    }

    #[tokio::test]
    async fn expired_url_is_refreshed_and_retried_once() {
        assert_eq!(run(vec![Err(AppError::UrlExpired), Ok(7)]).await, (Ok(7), 2, 2));
    }

    #[tokio::test]
    async fn second_expiry_is_not_retried_again() {
        let outcomes = vec![Err(AppError::UrlExpired), Err(AppError::UrlExpired), Ok(7)];
        assert_eq!(run(outcomes).await, (Err(AppError::UrlExpired), 2, 2));
    }

    #[tokio::test]
    async fn other_errors_are_not_retried() {
        assert_eq!(run(vec![Err(AppError::Timeout), Ok(7)]).await, (Err(AppError::Timeout), 1, 1));
    }

    #[tokio::test]
    async fn failure_to_get_a_fresh_url_is_reported() {
        let calls = AtomicUsize::new(0);
        let result: Result<(), AppError> = with_fresh_url(
            || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                async move { if n == 0 { Ok(signed(0)) } else { Err(AppError::Network) } }
            },
            |_| async { Err(AppError::UrlExpired) },
        )
        .await;
        assert_eq!(result, Err(AppError::Network));
    }

    #[test]
    fn download_qualities_round_trip_through_their_api_names() {
        for quality in DownloadQuality::ALL {
            assert_eq!(DownloadQuality::parse(quality.as_str()), Some(quality));
            assert_eq!(serde_json::to_string(&quality).unwrap(), format!("\"{}\"", quality.as_str()));
        }
        assert_eq!(DownloadQuality::parse("hires"), None);
    }

    #[test]
    fn a_lower_quality_than_asked_is_reported() {
        let done =
            |delivered| Downloaded { path: "x.m4a".into(), delivered, asked: DownloadQuality::Lossless };
        assert!(done(Some(DownloadQuality::Medium)).is_lower_than_asked());
        assert!(!done(Some(DownloadQuality::Lossless)).is_lower_than_asked());
        assert!(!done(None).is_lower_than_asked(), "an older server does not say: nothing to report");
    }

    #[test]
    fn download_filename_matches_server_rule() {
        assert_eq!(download_filename("Low \"Tide\"/../x", "flac"), "Low Tidex.flac");
    }

    #[test]
    fn download_filename_falls_back_to_track() {
        assert_eq!(download_filename("城市", "ogg"), "track.ogg");
    }

    // ---------- Test contro un server reale (cargo test -- --ignored) ----------

    fn live_api() -> ApiClient {
        let url = std::env::var("PSYTIFY_TEST_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
        ApiClient::new(&url).unwrap()
    }

    async fn live_tracks(api: &ApiClient) -> Vec<Track> {
        let query = TrackQuery { q: None, sort: client::Sort::Title, limit: 100, offset: 0 };
        let tracks = api.tracks(query).await.unwrap().items;
        assert!(!tracks.is_empty(), "the server catalogue is empty");
        tracks
    }

    #[tokio::test]
    #[ignore = "needs a running Psytify server with published tracks"]
    async fn live_audio_of_every_published_format_decodes() {
        let api = live_api();
        let mut formats = std::collections::BTreeSet::new();
        for track in live_tracks(&api).await {
            for quality in [Quality::Lossless, Quality::Standard] {
                let bytes = load_audio(api.clone(), track.id, quality).await.unwrap();
                let decoder = rodio::Decoder::new(std::io::Cursor::new(bytes));
                assert!(decoder.is_ok(), "{} ({quality:?}) does not decode", track.title);
            }
            formats.insert(track.audio_format);
        }
        println!("decoded formats: {formats:?}");
    }

    #[tokio::test]
    #[ignore = "needs a running Psytify server with published tracks"]
    async fn live_a_warm_track_starts_without_waiting() {
        let api = live_api();
        let tracks = live_tracks(&api).await;
        let (cold_track, warm_track) = (&tracks[0], &tracks[1]);

        let started = std::time::Instant::now();
        let cold = stream_audio(api.clone(), cold_track.id, Quality::Lossless).await.unwrap();
        let cold_time = started.elapsed();
        cold.cancel();

        let started = std::time::Instant::now();
        let warm = warm_audio(api.clone(), warm_track.id, Quality::Lossless).await.unwrap();
        let warming = started.elapsed();
        // Il clic arriva dopo: la testata è già in memoria.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let started = std::time::Instant::now();
        warm.release();
        assert!(warm.ready(crate::player::stream::PREFETCH_BYTES).await);
        let warm_time = started.elapsed();
        warm.cancel();

        println!("cold start {cold_time:?}; warming took {warming:?}; start after warming {warm_time:?}");
        assert!(warm_time < Duration::from_millis(50), "{warm_time:?}");
    }

    #[tokio::test]
    #[ignore = "needs a running Psytify server with an Internet Archive track"]
    async fn live_archive_flac_seeks_to_a_minute() {
        use rodio::Source;
        let api = live_api();
        let mut found = None;
        for track in live_tracks(&api).await.into_iter().filter(|t| t.duration() > Duration::from_secs(90)) {
            let url = api.stream_url(track.id, Quality::Lossless).await.unwrap();
            if url.url.contains("archive.org") && track.audio_format == "flac" {
                found = Some((track, url));
                break;
            }
        }
        let (track, url) = found.expect("an Archive FLAC longer than 90 s in the catalogue");
        let started = std::time::Instant::now();
        let stream = api.open_stream(&url).await.unwrap();
        let opened = started.elapsed();
        let seek = tokio::task::spawn_blocking(move || {
            let mut source =
                crate::player::decoder::SymphoniaSource::open(Box::new(stream.reader())).unwrap();
            let before = std::time::Instant::now();
            source.try_seek(Duration::from_secs(60)).unwrap();
            let first = source.next();
            stream.cancel();
            (before.elapsed(), first.is_some())
        });
        let (seeked, played) = seek.await.unwrap();
        println!("{}: opened in {opened:?}, seek to 1:00 played after {seeked:?}", track.title);
        assert!(played);
    }

    #[tokio::test]
    #[ignore = "needs a running Psytify server with published tracks"]
    async fn live_rejected_signature_is_refreshed_once() {
        let api = live_api();
        let track = live_tracks(&api).await.remove(0);
        let requests = AtomicUsize::new(0);
        let bytes = with_fresh_url(
            || {
                let api = api.clone();
                let n = requests.fetch_add(1, Ordering::SeqCst);
                async move {
                    let mut url = api.stream_url(track.id, Quality::Lossless).await?;
                    if n == 0 {
                        // Firma alterata: lo storage risponde 403 come per un URL scaduto.
                        url.url = url.url.replace("X-Amz-Signature=", "X-Amz-Signature=00");
                    }
                    Ok(url)
                }
            },
            |url| {
                let api = api.clone();
                async move { api.fetch_audio(&url).await }
            },
        )
        .await
        .unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(requests.into_inner(), 2);
    }

    #[tokio::test]
    #[ignore = "needs a running Psytify server with published tracks"]
    async fn live_search_albums_and_covers_parse() {
        let api = live_api();
        let track = live_tracks(&api).await.remove(0);
        let results = api.search(&track.title).await.unwrap();
        assert!(results.top.is_some() && !results.tracks.is_empty(), "{results:?}");
        // Un refuso nel nome dell'artista trova comunque i suoi brani.
        let typo = format!("{}x", track.artist.name);
        let results = api.search(&typo).await.unwrap();
        assert!(!results.tracks.is_empty(), "{typo}: {results:?}");
        api.albums(client::Sort::Recent, 10, 0).await.unwrap();
        if let Some(url) = results.tracks.iter().find_map(|t| t.cover_url.clone()) {
            assert!(!api.cover(&url).await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    #[ignore = "needs a running Psytify server with published tracks"]
    async fn live_download_is_refused_for_non_downloadable_track() {
        let api = live_api();
        let Some(track) = live_tracks(&api).await.into_iter().find(|t| !t.downloadable) else { return };
        let err = api.download_url(track.id, DownloadQuality::Lossless).await.unwrap_err();
        assert!(matches!(err, AppError::Api { status: 403, ref code, .. } if code == "not_downloadable"));
    }

    #[tokio::test]
    #[ignore = "needs a running Psytify server with published tracks"]
    async fn live_download_writes_the_file_without_overwriting() {
        let api = live_api();
        let track = live_tracks(&api).await.into_iter().find(|t| t.downloadable).unwrap();
        let dir = std::env::temp_dir().join(format!("psytify-dl-{}", Uuid::new_v4()));

        let first = download_track(api.clone(), track.clone(), dir.clone(), DownloadQuality::Lossless)
            .await
            .unwrap()
            .path;
        let second = download_track(api.clone(), track.clone(), dir.clone(), DownloadQuality::Lossless)
            .await
            .unwrap()
            .path;

        // Il download è la piena qualità: lo stesso file dell'ascolto lossless.
        let expected = load_audio(api, track.id, Quality::Lossless).await.unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), expected.to_vec());
        assert_ne!(first, second, "the second download must not overwrite the first");
        assert!(!dir.join(format!("{}.part", first.file_stem().unwrap().to_string_lossy())).exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
