//! Catalog models, as described in docs/BACKEND.md.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: Uuid,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Licence {
    pub kind: String,
    pub attribution: String,
}

impl Licence {
    /// CC licences with attribution (also NC and ND) require showing the rights holder.
    pub fn requires_attribution(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "cc-by" | "cc-by-sa" | "cc-by-nc" | "cc-by-nc-sa" | "cc-by-nd" | "cc-by-nc-nd"
        )
    }

    pub fn label(&self) -> &'static str {
        match self.kind.as_str() {
            "owned" => "Proprietà dell'artista",
            "cc0" => "CC0",
            "cc-by" => "CC BY",
            "cc-by-sa" => "CC BY-SA",
            "licensed-agreement" => "Su licenza",
            "external" => "Fonte esterna · solo ascolto",
            "cc-by-nc" => "CC BY-NC",
            "cc-by-nc-sa" => "CC BY-NC-SA",
            "cc-by-nd" => "CC BY-ND",
            "cc-by-nc-nd" => "CC BY-NC-ND",
            "public-domain" => "Pubblico dominio",
            "live-trade" => "Concerto dal vivo · solo ascolto",
            _ => "Licenza",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: Uuid,
    pub title: String,
    pub track_no: Option<u32>,
    pub duration_ms: u64,
    pub audio_format: String,
    pub downloadable: bool,
    /// The track has a music video. Absent in playlists saved before the field: `false`.
    #[serde(default)]
    pub has_video: bool,
    /// The music video is already in the server's storage; if `false` the first `video-url` downloads it.
    #[serde(default)]
    pub video_ready: bool,
    pub artist: ArtistRef,
    pub album: Option<AlbumRef>,
    pub licence: Licence,
    /// Public cover (Internet Archive). Absent in saved playlists from before the field.
    #[serde(default)]
    pub cover_url: Option<String>,
    pub created_at: String,
}

impl Track {
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.duration_ms)
    }

    /// File extension for the download: only the known formats, otherwise `audio`.
    pub fn extension(&self) -> &str {
        match self.audio_format.as_str() {
            ext @ ("flac" | "mp3" | "ogg" | "wav" | "m4a") => ext,
            _ => "audio",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Album {
    pub id: Uuid,
    pub title: String,
    pub released_on: Option<String>,
    pub artist: ArtistRef,
    #[serde(default)]
    pub cover_url: Option<String>,
    pub tracks: Vec<Track>,
}

/// Albums in lists: catalog (`GET /v1/catalog/albums`) and search.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AlbumSummary {
    pub id: Uuid,
    pub title: String,
    pub released_on: Option<String>,
    pub artist: ArtistRef,
    pub track_count: u32,
    #[serde(default)]
    pub cover_url: Option<String>,
}

/// Artist in the search results.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ArtistSummary {
    pub id: Uuid,
    pub name: String,
    pub track_count: u32,
    pub album_count: u32,
    #[serde(default)]
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TopKind {
    Artist,
    Album,
    Track,
}

/// Main result: the first item of the list of its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct TopHit {
    pub kind: TopKind,
    pub id: Uuid,
}

/// `GET /v1/catalog/search`: main result, tracks, artists and albums.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct SearchResults {
    pub top: Option<TopHit>,
    pub tracks: Vec<Track>,
    pub artists: Vec<ArtistSummary>,
    pub albums: Vec<AlbumSummary>,
}

impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.artists.is_empty() && self.albums.is_empty()
    }
}

/// The external sources the server searches tracks from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    YouTube,
    SoundCloud,
    Bandcamp,
    /// The official catalog (Deezer): the server finds the audio when play is pressed.
    Deezer,
}

impl SourceKind {
    /// Name used by the backend in the path `/v1/sources/{source}/...`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::YouTube => "youtube",
            Self::SoundCloud => "soundcloud",
            Self::Bandcamp => "bandcamp",
            Self::Deezer => "deezer",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::SoundCloud => "SoundCloud",
            Self::Bandcamp => "Bandcamp",
            Self::Deezer => "Catalogo",
        }
    }
}

/// An external search result. `duration_ms` is missing when the source does not declare the duration
/// (Bandcamp): the server knows it only after opening the track. It is saved in the configuration
/// for the tracks of a playlist still to be imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceItem {
    pub source: SourceKind,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: Option<u64>,
    /// Official cover (only the Deezer catalog). Absent in playlists saved before the field.
    #[serde(default)]
    pub cover_url: Option<String>,
}

/// A YouTube playlist or a Bandcamp album found by the external search.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SourcePlaylist {
    pub source: SourceKind,
    /// For the Deezer catalog `album/123` or `playlist/456`.
    pub id: String,
    pub title: String,
    pub owner: String,
    #[serde(default)]
    pub cover_url: Option<String>,
    /// Only the catalog: `album`, `single`, `ep` or `playlist`.
    #[serde(default)]
    pub kind: String,
    /// Release date (`2003-03-25`), if known.
    #[serde(default)]
    pub released_on: Option<String>,
}

impl SourcePlaylist {
    /// "Album", "Singolo", "EP" or "Playlist": how it appears in the sheets.
    pub fn kind_label(&self) -> &'static str {
        match (self.source, self.kind.as_str()) {
            (SourceKind::Bandcamp, _) | (SourceKind::Deezer, "album") => "Album",
            (SourceKind::Deezer, "single") => "Singolo",
            (SourceKind::Deezer, "ep") => "EP",
            _ => "Playlist",
        }
    }

    /// The release year, if known.
    pub fn year(&self) -> Option<&str> {
        self.released_on.as_deref().and_then(|date| date.get(..4))
    }
}

/// The tracks of an external playlist, in the source's order: each one plays by importing it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SourcePlaylistPage {
    pub source: SourceKind,
    pub id: String,
    pub title: String,
    pub owner: String,
    pub items: Vec<SourceItem>,
}

/// An artist of the Deezer catalog.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MetaArtist {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub fans: u64,
}

#[derive(Debug, Deserialize)]
struct MetaName {
    name: String,
}

#[derive(Debug, Deserialize)]
struct MetaTrackDto {
    id: u64,
    title: String,
    artist: MetaName,
    duration_ms: u64,
    #[serde(default)]
    cover_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MetaAlbumDto {
    id: u64,
    title: String,
    artist: MetaName,
    #[serde(default)]
    cover_url: Option<String>,
    #[serde(default)]
    released_on: Option<String>,
    kind: String,
}

#[derive(Debug, Deserialize)]
struct MetaPlaylistDto {
    id: u64,
    title: String,
    owner: String,
    #[serde(default)]
    cover_url: Option<String>,
}

impl From<MetaTrackDto> for SourceItem {
    fn from(t: MetaTrackDto) -> Self {
        Self {
            source: SourceKind::Deezer,
            id: t.id.to_string(),
            title: t.title,
            artist: t.artist.name,
            duration_ms: Some(t.duration_ms),
            cover_url: t.cover_url,
        }
    }
}

impl From<MetaAlbumDto> for SourcePlaylist {
    fn from(a: MetaAlbumDto) -> Self {
        Self {
            source: SourceKind::Deezer,
            id: format!("album/{}", a.id),
            title: a.title,
            owner: a.artist.name,
            cover_url: a.cover_url,
            kind: a.kind,
            released_on: a.released_on,
        }
    }
}

impl From<MetaPlaylistDto> for SourcePlaylist {
    fn from(p: MetaPlaylistDto) -> Self {
        Self {
            source: SourceKind::Deezer,
            id: format!("playlist/{}", p.id),
            title: p.title,
            owner: p.owner,
            cover_url: p.cover_url,
            kind: "playlist".into(),
            released_on: None,
        }
    }
}

/// `GET /v1/meta/search`: tracks, artists, albums and playlists of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetaSearch {
    pub tracks: Vec<SourceItem>,
    pub artists: Vec<MetaArtist>,
    /// Albums and playlists, in order: albums first.
    pub lists: Vec<SourcePlaylist>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MetaSearchDto {
    tracks: Vec<MetaTrackDto>,
    artists: Vec<MetaArtist>,
    albums: Vec<MetaAlbumDto>,
    playlists: Vec<MetaPlaylistDto>,
}

impl From<MetaSearchDto> for MetaSearch {
    fn from(dto: MetaSearchDto) -> Self {
        let lists = dto
            .albums
            .into_iter()
            .map(SourcePlaylist::from)
            .chain(dto.playlists.into_iter().map(SourcePlaylist::from))
            .collect();
        Self { tracks: dto.tracks.into_iter().map(Into::into).collect(), artists: dto.artists, lists }
    }
}

/// The page of a catalog artist: the popular tracks and the whole discography.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaArtistPage {
    pub artist: MetaArtist,
    pub top: Vec<SourceItem>,
    pub albums: Vec<SourcePlaylist>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MetaArtistPageDto {
    artist: MetaArtist,
    top: Vec<MetaTrackDto>,
    albums: Vec<MetaAlbumDto>,
}

impl From<MetaArtistPageDto> for MetaArtistPage {
    fn from(dto: MetaArtistPageDto) -> Self {
        Self {
            artist: dto.artist,
            top: dto.top.into_iter().map(Into::into).collect(),
            albums: dto.albums.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct MetaListPageDto {
    #[serde(alias = "album", alias = "playlist")]
    head: MetaListHead,
    tracks: Vec<MetaTrackDto>,
}

#[derive(Debug, Deserialize)]
struct MetaListHead {
    id: u64,
    title: String,
    #[serde(default)]
    artist: Option<MetaName>,
    #[serde(default)]
    owner: Option<String>,
}

impl MetaListPageDto {
    pub(super) fn into_page(self, kind: &str) -> SourcePlaylistPage {
        let owner = self.head.artist.map(|a| a.name).or(self.head.owner).unwrap_or_default();
        SourcePlaylistPage {
            source: SourceKind::Deezer,
            id: format!("{kind}/{}", self.head.id),
            title: self.head.title,
            owner,
            items: self.tracks.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenedArtist {
    pub id: Uuid,
}

/// Track lyrics from LRCLIB, quoted by the backend. `synced` is in LRC format.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Lyrics {
    pub track_id: Uuid,
    pub found: bool,
    pub synced: Option<String>,
    pub plain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub limit: i64,
    pub offset: i64,
    pub next_offset: Option<i64>,
}

/// Listening quality chosen in the settings (`?quality=` of `stream-url`): lossless takes the
/// FLAC or the WAV (Internet Archive; no separate hi-res tier), standard the YouTube version.
/// If it is missing, the server gives the other one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    #[default]
    Lossless,
    Standard,
}

impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lossless => "lossless",
            Self::Standard => "standard",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Lossless => "Lossless",
            Self::Standard => "Standard",
        }
    }
}

/// Quality of a download (`?quality=` of `download-url`). `Lossless` is the best the track has
/// (FLAC or WAV, with no separate hi-res tier); the others are lossy versions, smaller.
/// The server never improves a lossy file: what actually arrives says so in `SignedUrl::quality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadQuality {
    #[default]
    Lossless,
    High,
    Medium,
    Low,
}

impl DownloadQuality {
    /// From the highest to the lowest, as the choice shows them.
    pub const ALL: [Self; 4] = [Self::Lossless, Self::High, Self::Medium, Self::Low];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lossless => "lossless",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|quality| quality.as_str() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Lossless => "Lossless",
            Self::High => "Alta",
            Self::Medium => "Media",
            Self::Low => "Bassa",
        }
    }

    /// What kind of file it is, in a few words: it sits in a line of the menu.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Lossless => "FLAC o WAV",
            Self::High => "AAC 256 kbps",
            Self::Medium => "AAC 128 kbps",
            Self::Low => "AAC 64 kbps",
        }
    }

    /// The choice explained for the settings, with the space a four-minute track takes.
    pub fn description(self) -> &'static str {
        match self {
            Self::Lossless => "FLAC o WAV senza perdita: il file originale, circa 25 MB a brano",
            Self::High => "AAC 256 kbps: alta qualità, circa 8 MB a brano",
            Self::Medium => "AAC 128 kbps: buona qualità, circa 4 MB a brano",
            Self::Low => "AAC 64 kbps: file piccolo, circa 2 MB a brano",
        }
    }
}

/// A search result on Internet Archive.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiscoverItem {
    pub identifier: String,
    pub title: String,
    pub creator: Option<String>,
    pub licence: String,
    pub year: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    /// The item has at least one FLAC or a WAV.
    #[serde(default)]
    pub lossless: bool,
}

/// An Internet Archive library: the collection of a netlabel.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiscoverLibrary {
    pub identifier: String,
    pub title: String,
    pub creator: Option<String>,
    /// Albums from the search that are in the library.
    pub found: u32,
    #[serde(default)]
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiscoverPage {
    pub items: Vec<DiscoverItem>,
    #[serde(default)]
    pub libraries: Vec<DiscoverLibrary>,
}

/// A page of a library's albums.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LibraryPage {
    pub items: Vec<DiscoverItem>,
    pub next_page: Option<u32>,
}

/// Tracks of an item just brought into the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiscoverImport {
    pub album: Option<AlbumRef>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ArtistAlbum {
    pub id: Uuid,
    pub title: String,
    pub released_on: Option<String>,
    pub track_count: u32,
    #[serde(default)]
    pub cover_url: Option<String>,
}

/// Page of an artist: published albums and tracks.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ArtistPage {
    pub id: Uuid,
    pub name: String,
    /// Official photo of the artist; without it, the cover of one of their albums or tracks is used.
    #[serde(default)]
    pub image_url: Option<String>,
    pub albums: Vec<ArtistAlbum>,
    pub tracks: Vec<Track>,
}

/// The user signed in, from `GET /v1/me`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Me {
    pub user_id: String,
    pub admin: bool,
    /// `None` until the user has a username: the social functions ask for it.
    #[serde(default)]
    pub profile: Option<super::social::MyProfile>,
}

/// Signed URL. `Debug` does not print the URL: it is a temporary secret.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct SignedUrl {
    pub kind: String,
    pub url: String,
    pub expires_at: String,
    /// Music videos only: the height of the version served (360 the first, 720 the full one).
    #[serde(default)]
    pub height: Option<u32>,
    /// Downloads only: the file format received (`flac`, `wav`, `mp3`, `ogg`, `m4a`).
    #[serde(default)]
    pub format: Option<String>,
    /// Downloads only: the quality actually received (see `DownloadQuality::parse`).
    #[serde(default)]
    pub quality: Option<String>,
}

impl std::fmt::Debug for SignedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedUrl")
            .field("kind", &self.kind)
            .field("url", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("quality", &self.quality)
            .finish()
    }
}

/// The backend's uniform error body: the client needs only the stable code, the message
/// shown is translated into Italian starting from the code.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiErrorBody {
    pub error: String,
}

#[cfg(any(test, feature = "test-support"))]
pub mod fixtures {
    use super::*;

    pub fn track(n: u128) -> Track {
        Track {
            id: Uuid::from_u128(n),
            title: format!("Brano {n}"),
            track_no: u32::try_from(n).ok(),
            duration_ms: 180_000,
            audio_format: "flac".into(),
            downloadable: false,
            has_video: false,
            video_ready: false,
            artist: ArtistRef { id: Uuid::from_u128(1000 + n % 3), name: format!("Artista {}", n % 3) },
            album: n
                .is_multiple_of(2)
                .then(|| AlbumRef { id: Uuid::from_u128(2000 + n % 4), title: format!("Album {}", n % 4) }),
            licence: Licence { kind: "cc-by".into(), attribution: "Titolare".into() },
            cover_url: None,
            created_at: "2026-10-01T09:30:00Z".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_parses_documented_example() {
        let json = r#"{
          "id": "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b", "title": "Low Tide", "track_no": 1,
          "duration_ms": 215000, "audio_format": "flac", "downloadable": true,
          "artist": {"id": "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", "name": "Marea"},
          "album": null, "licence": {"kind": "cc-by", "attribution": "Marea"},
          "created_at": "2026-10-01T09:30:00Z"
        }"#;
        let track: Track = serde_json::from_str(json).unwrap();
        assert!(track.album.is_none() && track.licence.requires_attribution());
    }

    #[test]
    fn track_saved_before_videos_has_no_video() {
        let json = r#"{
          "id": "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b", "title": "x", "track_no": null,
          "duration_ms": 1, "audio_format": "flac", "downloadable": false,
          "artist": {"id": "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", "name": "y"},
          "album": null, "licence": {"kind": "cc0", "attribution": "z"}, "created_at": ""
        }"#;
        assert!(!serde_json::from_str::<Track>(json).unwrap().has_video);
    }

    #[test]
    fn search_results_parse_documented_example() {
        let json = r#"{
          "top": {"kind": "artist", "id": "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d"},
          "tracks": [],
          "artists": [{"id": "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", "name": "Marea", "track_count": 3,
                       "album_count": 1, "cover_url": "https://archive.org/services/img/marea-banchi"}],
          "albums": [{"id": "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e", "title": "Banchi di sabbia", "released_on": null,
                      "artist": {"id": "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", "name": "Marea"}, "track_count": 3,
                      "cover_url": "https://archive.org/services/img/marea-banchi"}]
        }"#;
        let results: SearchResults = serde_json::from_str(json).unwrap();
        assert_eq!(results.top.map(|t| t.kind), Some(TopKind::Artist));
        assert!(!results.is_empty());
    }

    #[test]
    fn quality_serializes_like_the_api_parameter() {
        assert_eq!(serde_json::to_string(&Quality::Standard).unwrap(), r#""standard""#);
    }

    #[test]
    fn signed_url_debug_hides_the_url() {
        let url = SignedUrl {
            kind: "stream".into(),
            url: "https://s/secret?X-Amz-Signature=abc".into(),
            expires_at: String::new(),
            height: None,
            format: None,
            quality: None,
        };
        assert!(!format!("{url:?}").contains("Signature"));
    }

    #[test]
    fn unknown_format_has_generic_extension() {
        let mut track = fixtures::track(1);
        track.audio_format = "../../evil".into();
        assert_eq!(track.extension(), "audio");
    }
}
