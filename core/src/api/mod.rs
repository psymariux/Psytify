//! Access to the backend: HTTP client, models and flows that combine several requests.

pub mod client;
pub mod models;
pub mod social;
pub mod transfer;
pub mod update;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

use uuid::Uuid;

pub use client::{ApiClient, DownloadReply, Sort, TrackQuery, base_host};
pub use models::{
    Album, AlbumSummary, ArtistPage, ArtistSummary, DiscoverImport, DiscoverItem, DiscoverLibrary,
    DiscoverPage, DownloadQuality, LibraryPage, Lyrics, Me, MetaArtist, MetaArtistPage, MetaSearch, Page,
    Quality, SearchResults, SignedUrl, SourceItem, SourceKind, SourcePlaylist, SourcePlaylistPage, TopKind,
    Track,
};
pub use transfer::{Cancel, Progress};

use crate::error::AppError;
use crate::player::decoder;
use crate::player::stream::{StreamBuffer, WARM_BYTES};

/// A name without spaces or punctuation, in lowercase, to compare titles and artists.
fn compact(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Among the catalog's tracks, the one that is the same as the saved track: same artist and title, and
/// the closest duration (within 15 seconds, if it knows it).
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

/// A saved track that the server no longer knows (database recreated): it finds it again in the
/// official catalog and has it imported again. Returns the new track, with its identifier.
pub async fn heal_track(api: ApiClient, track: Track) -> Result<Track, AppError> {
    let query = format!("{} {}", track.artist.name, track.title);
    let found = api.meta_search(&query).await?;
    let item = best_match(&track, &found.tracks).ok_or(AppError::InvalidResponse)?;
    let imported = api.import_source(item.source, &item.id, false).await?;
    imported.tracks.into_iter().next().ok_or(AppError::InvalidResponse)
}

/// Requests a signed URL and uses it. If the storage answers 401 or 403 the URL has expired: it
/// requests a new one and retries only once.
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

/// Opens the progressive listening of a track in the chosen quality: the buffer already has the first bytes and
/// keeps filling. An MP4 (M4A) with the index at the end must be read from the end: if the server
/// did not say the length, the whole file is waited for.
pub async fn stream_audio(
    api: ApiClient,
    track_id: Uuid,
    quality: Quality,
) -> Result<StreamBuffer, AppError> {
    open_audio(api, track_id, quality, None).await
}

/// Opens a track that may be listened to: it downloads only the head (`WARM_BYTES`) and stops. If
/// it then plays, `StreamBuffer::release` (or the first byte read beyond the head) makes it continue.
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
        // An M4A without a length decodes only with the whole file: no lone head.
        stream.release();
        // `ready(usize::MAX)` waits for the end of the download, or `false` if it failed.
        if !stream.ready(usize::MAX).await {
            return Err(AppError::AudioDecode);
        }
    }
    Ok(stream)
}

/// Audio bytes of a track in the chosen quality, all in memory (tests against the live server).
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

/// How many times a version being prepared is requested before giving up: the server re-encodes
/// for three minutes at most.
const PREPARE_ATTEMPTS: usize = 90;

/// A downloaded track: where it ended up, what quality it really has and what file it is (weight and SHA-256 of the
/// content, computed while it was written: it serves to recognise a changed or damaged file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    pub path: PathBuf,
    pub size: u64,
    /// SHA-256 of the content, in hexadecimal.
    pub sha256: String,
    /// The file extension the server gave.
    pub format: String,
    /// The quality the server gave; `None` if it did not say (an older server).
    pub delivered: Option<DownloadQuality>,
    /// The quality requested.
    pub asked: DownloadQuality,
}

impl Downloaded {
    /// The track does not have the quality requested (a lossy file is not improved): the nearest one came.
    pub fn is_lower_than_asked(&self) -> bool {
        self.delivered.is_some_and(|got| got != self.asked)
    }
}

/// Requests the download URL in the chosen quality, waiting for the server to finish preparing the
/// version if a re-encoding is needed.
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

/// Downloads a track into the given folder, in the requested quality, and returns the path of the
/// file. The extension is that of the file the server really gives (a re-encoded version is m4a).
/// `progress` says where it is. The file is born with a unique temporary name and takes its own only at the
/// end of the download, without ever covering an existing file.
pub async fn download_track(
    api: ApiClient,
    track: Track,
    dir: PathBuf,
    quality: DownloadQuality,
    progress: Arc<Progress>,
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
            let (dir, track, progress) = (dir.clone(), track.clone(), Arc::clone(&progress));
            async move {
                let extension =
                    url.format.as_deref().filter(|f| is_known_extension(f)).unwrap_or(track.extension());
                let fetched = api.fetch_into(&url.url, &dir, &progress).await?;
                let (size, sha256) = (fetched.size, fetched.sha256.clone());
                let filename = download_filename(&track.title, extension);
                let path = fetched
                    .temp
                    .publish(&dir, &filename, false)
                    .await
                    .map_err(|_| AppError::Io(dir.display().to_string()))?;
                let delivered = url.quality.as_deref().and_then(DownloadQuality::parse);
                Ok(Downloaded { path, size, sha256, format: extension.to_owned(), delivered, asked: quality })
            }
        },
    )
    .await
}

fn is_known_extension(extension: &str) -> bool {
    matches!(extension, "flac" | "mp3" | "ogg" | "wav" | "m4a")
}

/// Same rule as the server: ASCII letters and digits, spaces, hyphens and underscores, at most
/// 100 characters; if it stays empty, `track`.
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

/// Never overwrites an existing file: it adds " (2)", " (3)", ...
pub(crate) async fn unique_path(dir: &Path, filename: &str) -> PathBuf {
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

    /// Simulates backend and storage: counts the requests and answers with the outcomes given, in order.
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
        let done = |delivered| Downloaded {
            path: "x.m4a".into(),
            size: 1,
            sha256: String::new(),
            format: "m4a".into(),
            delivered,
            asked: DownloadQuality::Lossless,
        };
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

    // ---------- Tests against a real server (cargo test -- --ignored) ----------

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
        // The click comes later: the head is already in memory.
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
                        // Altered signature: the storage answers 403 as for an expired URL.
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
        // A typo in the artist's name still finds their tracks.
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

        let progress = Arc::new(Progress::default());
        let download = |api: ApiClient| {
            download_track(api, track.clone(), dir.clone(), DownloadQuality::Lossless, Arc::clone(&progress))
        };
        let first = download(api.clone()).await.unwrap().path;
        let second = download(api.clone()).await.unwrap().path;

        // The download is full quality: the same file as the lossless listening.
        let expected = load_audio(api, track.id, Quality::Lossless).await.unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), expected.to_vec());
        assert_ne!(first, second, "the second download must not overwrite the first");
        assert!(
            std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().ends_with(".part"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
