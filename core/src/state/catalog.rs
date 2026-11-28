//! Catalog (step 4.4): Home, search, the library list, albums and artists, Archive libraries
//! and the official catalog, plus the finding again of vanished tracks and the rereading of saved ones.
//!
//! The domain loads everything from the backend with jobs ([`Job`]) and publishes the state of each load
//! ([`Load`]) into the view: a frontend copies it and draws it, without opening requests. The album and
//! artist pages are kept in two caches of fixed capacity, with eviction of the least used ([`Lru`]); an
//! old page is shown at once and reread silently. The search waits 300 ms after the last key
//! with a core timer ([`Timer::Search`]), and only the last request counts: each key changes
//! the request number and the replies of a superseded number are discarded.
//!
//! The domain talks to the others through the passing that the core does after each input: what it
//! asks of the library (the Home's recent tracks the feed comes from, the correct covers, the
//! followed artists) and of playback (the tracks found again, the head of an album's first track)
//! goes into two stores ([`Catalog::take_library`], [`Catalog::take_playback`]), and what it
//! needs from them (the missing track, the track just started, the feed) is given to it by its functions
//! `heal`, `track_started` and `after_library`. The view changes only if something really changes: a
//! command equal to the previous one, or a superseded reply, publishes nothing.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::api::{
    self, Album, AlbumSummary, ApiClient, ArtistPage, DiscoverItem, DiscoverLibrary, DiscoverPage,
    LibraryPage, MetaArtist, MetaSearch, Page, SearchResults, Sort, SourceItem, SourcePlaylist, Track,
    TrackQuery,
};
use crate::catalog::{Load, clean_query, portrait_url};
use crate::config;
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::shared::Shared;
use crate::state::library::{LibraryCmd, LibraryView};
use crate::state::playback::PlaybackCmd;

/// Wait after the last key before launching the search.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// Tracks per shelf of the Home (alphabetical).
const HOME_SECTION_SIZE: u32 = 12;
/// Recent tracks from which releases, novelties and mixes are derived (the most the server gives).
const HOME_RELEASES: u32 = 100;
/// Recent albums shown on the Home.
const HOME_ALBUMS: u32 = 24;
/// Tracks per page of the library list.
const LIBRARY_PAGE: u32 = 50;
/// Albums shown in the library (a single page, from A to Z).
const LIBRARY_ALBUMS: u32 = 100;
/// Artist pages kept in memory: beyond that, the least used goes out.
pub const ARTIST_PAGES: usize = 40;
/// Album pages kept in memory: beyond that, the least used goes out.
pub const ALBUM_PAGES: usize = 24;
/// After how long a cached page is downloaded again (silently) when opened.
pub const STALE_AFTER: Duration = Duration::from_secs(90);
/// Artist pages loaded in advance at a time: abundance would cost the server.
const PREFETCH_BATCH: usize = 8;
/// Saved tracks without a cover reread from the server at startup, at most.
const STALE_REFRESH: usize = 24;
/// Reread tracks remembered for the session, so they are not reread twice.
const REFRESHED: usize = 256;
/// Events kept in the view: a frontend that reads at each publish does not lose any.
const EVENTS: usize = 32;

/// What is loaded or reloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// The three shelves of the Home (recent, alphabetical, albums).
    Home,
    /// The tracks and albums of the library list.
    Library,
    Album(Uuid),
    Artist(Uuid),
}

/// The domain's commands.
#[derive(Debug, Clone)]
pub enum CatalogCmd {
    /// Shows a resource: from the cache if there is one, otherwise from the network. If it is
    /// already ready or on its way it does nothing; on the loaded Home it rebuilds the feed if plays changed.
    Ensure(Resource),
    /// "Riprova": reloads from the network, even if it is in the cache.
    Retry(Resource),
    /// The next page of the library's tracks.
    LibraryMore,
    /// Changes the library's order and reloads it.
    LibrarySort(Sort),
    /// The search text has changed: it starts after [`SEARCH_DEBOUNCE`] from the last key.
    SearchInput(String),
    /// Searches this text at once (at startup, or "Riprova").
    Search(String),
    ArchiveLibraryOpen(DiscoverLibrary),
    ArchiveLibraryMore,
    ArchiveLibraryClose,
    /// Brings the official catalog's artist with this identifier into the server.
    OpenMetaArtist(u64),
    /// The catalog has changed (an import): Home and library reload at the next visit.
    Changed,
}

/// The three sections of the Home.
#[derive(Debug, Clone, Default)]
pub struct HomeView {
    pub recent: Load<Arc<[Track]>>,
    pub alphabetical: Load<Arc<[Track]>>,
    pub albums: Load<Arc<[AlbumSummary]>>,
}

/// The library list: tracks in pages and albums in a single page.
#[derive(Debug, Clone, Default)]
pub struct LibraryListing {
    pub tracks: Load<Shared<Track>>,
    pub next_offset: Option<i64>,
    pub loading_more: bool,
    /// The error of a next page: the tracks already loaded stay.
    pub more_error: Option<AppError>,
    /// Order of albums and tracks: the most recent or from A to Z.
    pub sort: Sort,
    pub albums: Load<Arc<[AlbumSummary]>>,
    /// The catalog has more albums than those loaded.
    pub albums_truncated: bool,
}

/// An open Archive library: its albums, one page after another.
#[derive(Debug, Clone)]
pub struct OpenLibrary {
    pub library: DiscoverLibrary,
    /// First page loading or failed; then all the albums loaded so far.
    pub items: Load<Arc<[DiscoverItem]>>,
    pub next_page: Option<u32>,
    pub loading_more: bool,
}

impl OpenLibrary {
    /// Receives a page of albums: the first replaces the loading (or becomes the error
    /// shown in place of the albums), the others are added at the end. Returns the error of a
    /// next page, to be shown as a notice: the albums already loaded stay and "Altri album" can be
    /// pressed again.
    pub fn receive(&mut self, page: u32, result: Result<LibraryPage, AppError>) -> Result<(), AppError> {
        self.loading_more = false;
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(err) if page == 1 => {
                self.items = Load::Failed(err);
                return Ok(());
            }
            Err(err) => return Err(err),
        };
        self.next_page = loaded.next_page;
        let mut items: Vec<DiscoverItem> = match &self.items {
            Load::Ready(earlier) if page > 1 => earlier.to_vec(),
            _ => Vec::new(),
        };
        items.extend(loaded.items);
        self.items = Load::Ready(Arc::from(items));
        Ok(())
    }
}

/// The search results: the catalog, Internet Archive and the official catalog.
#[derive(Debug, Clone, Default)]
pub struct SearchView {
    /// The text searched last, already cleaned; empty if nothing is searched.
    pub submitted: String,
    pub results: Load<Arc<SearchResults>>,
    /// Results from Internet Archive, for what is not yet in the catalog.
    pub discover: Load<Arc<[DiscoverItem]>>,
    /// Archive libraries (netlabels) where the albums found are.
    pub libraries: Arc<[DiscoverLibrary]>,
    /// Library opened from the search: in place of Archive's results its albums are seen.
    pub open_library: Option<OpenLibrary>,
    /// Tracks of the official catalog (Deezer).
    pub sources: Load<Arc<[SourceItem]>>,
    /// Albums and playlists of the official catalog.
    pub playlists: Load<Arc<[SourcePlaylist]>>,
    /// Artists of the official catalog.
    pub artists: Arc<[MetaArtist]>,
}

/// Something the frontend must make known or do.
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogEvent {
    /// The official catalog's artist now has a page: the frontend opens it.
    MetaArtistOpened(Uuid),
    MetaArtistFailed(AppError),
    /// A page of albums from an Archive library did not arrive: a notice, the albums stay.
    LibraryPageFailed(AppError),
    /// A track found again or reread from the server: its cover may be new.
    Replaced(Arc<Track>),
    /// An artist's page has arrived in advance (or updated): its first covers can be
    /// downloaded at once, so opening it is immediate.
    Prefetched(Arc<ArtistPage>),
}

/// A [`CatalogEvent`] with its sequence number, which grows with each event.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogRaised {
    pub seq: u64,
    pub event: CatalogEvent,
}

/// What the frontends see of the domain.
#[derive(Debug, Clone, Default)]
pub struct CatalogView {
    pub home: HomeView,
    pub library: LibraryListing,
    pub search: SearchView,
    /// The open album and its loading.
    pub album: Option<(Uuid, Load<Arc<Album>>)>,
    /// The open artist and its loading.
    pub artist: Option<(Uuid, Load<Arc<ArtistPage>>)>,
    /// The latest events, from the oldest: a frontend shows those with a new `seq`, in order. If it
    /// forgets at most [`EVENTS`] between one read and the next, that is, never.
    pub events: Arc<VecDeque<CatalogRaised>>,
}

/// The domain's timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Expires [`SEARCH_DEBOUNCE`] after the last key: the search starts.
    Search,
}

/// Which Home shelf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Recent,
    Alphabetical,
}

/// How an artist page is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fetch {
    /// Opened by whoever is looking: if it is the one open, it replaces it, and if it fails it shows it.
    Open,
    /// In advance or to update silently: it goes into the cache, and an error is not shown.
    Background,
}

/// The domain's async jobs (network), which the host runs on the runtime. Those that fill a
/// part of the view carry its epoch or request number, to discard the replies that are
/// superseded.
#[derive(Debug)]
pub(crate) enum Job {
    Home {
        api: Result<ApiClient, AppError>,
        epoch: u64,
        section: Section,
    },
    HomeAlbums {
        api: Result<ApiClient, AppError>,
        epoch: u64,
    },
    Search {
        api: Result<ApiClient, AppError>,
        ticket: u64,
        query: String,
    },
    Discover {
        api: Result<ApiClient, AppError>,
        ticket: u64,
        query: String,
    },
    Meta {
        api: Result<ApiClient, AppError>,
        ticket: u64,
        query: String,
    },
    LibraryTracks {
        api: Result<ApiClient, AppError>,
        epoch: u64,
        sort: Sort,
        offset: u32,
        append: bool,
    },
    LibraryAlbums {
        api: Result<ApiClient, AppError>,
        epoch: u64,
        sort: Sort,
    },
    Album {
        api: Result<ApiClient, AppError>,
        id: Uuid,
        silent: bool,
    },
    Artist {
        api: Result<ApiClient, AppError>,
        id: Uuid,
        fetch: Fetch,
    },
    ArchiveLibrary {
        api: Result<ApiClient, AppError>,
        identifier: String,
        page: u32,
    },
    MetaArtist {
        api: Result<ApiClient, AppError>,
        id: u64,
    },
    /// The track no longer exists on the server: it searches the official catalog and imports it again.
    Heal {
        api: Result<ApiClient, AppError>,
        track: Arc<Track>,
    },
    /// Rereads a saved track from the server.
    Refresh {
        api: Result<ApiClient, AppError>,
        id: Uuid,
    },
}

/// The outcome of a [`Job`], which goes back to the domain.
#[derive(Debug)]
pub(crate) enum Done {
    Home { epoch: u64, section: Section, result: Result<Page<Track>, AppError> },
    HomeAlbums { epoch: u64, result: Result<Page<AlbumSummary>, AppError> },
    Search { ticket: u64, result: Result<Box<SearchResults>, AppError> },
    Discover { ticket: u64, result: Result<Box<DiscoverPage>, AppError> },
    Meta { ticket: u64, result: Result<Box<MetaSearch>, AppError> },
    LibraryTracks { epoch: u64, append: bool, result: Result<Page<Track>, AppError> },
    LibraryAlbums { epoch: u64, result: Result<Page<AlbumSummary>, AppError> },
    Album { id: Uuid, silent: bool, result: Result<Box<Album>, AppError> },
    Artist { id: Uuid, fetch: Fetch, result: Result<Box<ArtistPage>, AppError> },
    ArchiveLibrary { identifier: String, page: u32, result: Result<LibraryPage, AppError> },
    MetaArtist(Result<Uuid, AppError>),
    Healed { old: Uuid, result: Result<Box<Track>, AppError> },
    Refreshed(Result<Box<Track>, AppError>),
}

/// The outcome of a job that was lost.
fn lost<T>() -> Result<T, AppError> {
    Err(AppError::Network)
}

fn is_idle<T>(load: &Load<T>) -> bool {
    matches!(load, Load::Idle)
}

/// A cache of fixed capacity with eviction of the least used: whoever reads or writes a page brings it to
/// the end, and when full the one at the head goes out. It remembers when each page arrived, to say
/// whether it is old.
#[derive(Debug)]
pub(crate) struct Lru<T> {
    capacity: usize,
    /// From the least to the most used.
    entries: VecDeque<(Uuid, Instant, Arc<T>)>,
}

impl<T> Lru<T> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { capacity, entries: VecDeque::new() }
    }

    pub(crate) fn contains(&self, id: Uuid) -> bool {
        self.entries.iter().any(|(known, ..)| *known == id)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The page, and whether it is old (arrived more than [`STALE_AFTER`] ago). Having read it makes it the most
    /// recent.
    pub(crate) fn get(&mut self, id: Uuid, now: Instant) -> Option<(Arc<T>, bool)> {
        let at = self.entries.iter().position(|(known, ..)| *known == id)?;
        let entry = self.entries.remove(at)?;
        let found = (Arc::clone(&entry.2), now.saturating_duration_since(entry.1) > STALE_AFTER);
        self.entries.push_back(entry);
        Some(found)
    }

    /// Puts the page at the end (replacing the one with the same id) and discards the oldest beyond
    /// the capacity.
    pub(crate) fn put(&mut self, id: Uuid, value: T, now: Instant) -> Arc<T> {
        let value = Arc::new(value);
        self.entries.retain(|(known, ..)| *known != id);
        self.entries.push_back((id, now, Arc::clone(&value)));
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
        value
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

/// The domain's state: the published view and what stays inside the core.
#[derive(Debug)]
pub(crate) struct Catalog {
    pub(crate) view: Arc<CatalogView>,
    api: Result<ApiClient, AppError>,
    server: Option<String>,
    /// Changes when the catalog changes or the backend is another: the earlier replies are discarded.
    home_epoch: u64,
    library_epoch: u64,
    /// The search number: it grows at each key, and only the replies of the last one count.
    ticket: u64,
    /// The last text written, as it is (the frontend already has it in its field).
    draft: String,
    artists: Lru<ArtistPage>,
    albums: Lru<Album>,
    /// Artist pages arriving in the background.
    loading: HashSet<Uuid>,
    /// The number of the last event.
    events: u64,
    /// Tracks already reread in this session, the most recent at the end.
    refreshed: VecDeque<Uuid>,
    /// The last track loading seen starting.
    last_started: u64,
    /// Saved tracks without a cover must be reread: it is done once, at startup.
    startup: bool,
    /// The Home's artists must be loaded in advance, as soon as the library has their feed.
    prefetch_home: bool,
    /// The searched text has changed and the settings do not have it yet (`take_query`).
    query: Option<String>,
    /// What the domain asks of the library and of playback, waiting for the passing.
    library: Vec<LibraryCmd>,
    playback: Vec<PlaybackCmd>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Catalog {
    /// The domain with the backend chosen in the settings.
    pub(crate) fn new(server: Option<String>) -> Self {
        Self {
            view: Arc::default(),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            home_epoch: 0,
            library_epoch: 0,
            ticket: 0,
            draft: String::new(),
            artists: Lru::new(ARTIST_PAGES),
            albums: Lru::new(ALBUM_PAGES),
            loading: HashSet::new(),
            events: 0,
            refreshed: VecDeque::new(),
            last_started: 0,
            startup: false,
            prefetch_home: false,
            query: None,
            library: Vec::new(),
            playback: Vec::new(),
        }
    }

    fn view_mut(&mut self) -> &mut CatalogView {
        Arc::make_mut(&mut self.view)
    }

    /// The epoch of the Home's replies on the way (for the tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn home_epoch(&self) -> u64 {
        self.home_epoch
    }

    /// The epoch of the library list's replies on the way (for the tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn library_epoch(&self) -> u64 {
        self.library_epoch
    }

    fn api(&self) -> Result<ApiClient, AppError> {
        self.api.clone()
    }

    fn run(effects: &mut Vec<Effect>, job: Job) {
        effects.push(Effect::Run(effect::Job::Catalog(job)));
    }

    // ---------- Passing between domains ----------

    /// What the domain asks of the library, and empties it.
    pub(crate) fn take_library(&mut self) -> Vec<LibraryCmd> {
        std::mem::take(&mut self.library)
    }

    /// What the domain asks of playback, and empties it.
    pub(crate) fn take_playback(&mut self) -> Vec<PlaybackCmd> {
        std::mem::take(&mut self.playback)
    }

    /// The text searched last, if it has changed since the last delivery to the settings.
    pub(crate) fn take_query(&mut self) -> Option<String> {
        self.query.take()
    }

    /// The core has just started: the saved tracks without a cover are reread, at the first chance.
    pub(crate) fn start(&mut self) {
        self.startup = true;
    }

    /// The backend chosen in the settings has changed: the HTTP client is made again, the pages in memory
    /// belong to another server and are thrown away, the library list and the open pages are
    /// reloaded at the next visit, and the Home reloads at once.
    pub(crate) fn set_server(&mut self, server: &Option<String>, effects: &mut Vec<Effect>) {
        if self.server == *server {
            return;
        }
        self.server.clone_from(server);
        self.api = ApiClient::new(&config::api_url(server.as_deref()));
        self.artists.clear();
        self.albums.clear();
        self.loading.clear();
        self.library_epoch += 1;
        let view = self.view_mut();
        view.library = LibraryListing { sort: view.library.sort, ..LibraryListing::default() };
        view.album = None;
        view.artist = None;
        self.load_home(effects);
    }

    /// A track no longer exists on the server: it is searched for in the official catalog.
    pub(crate) fn heal(&mut self, track: Arc<Track>, effects: &mut Vec<Effect>) {
        Self::run(effects, Job::Heal { api: self.api(), track });
    }

    /// Another track has started playing: it is reread from the server, once per session (the
    /// saved copies may be from before the server had the music video or the official cover).
    /// ufficiale).
    pub(crate) fn track_started(&mut self, started: u64, current: Option<Uuid>, effects: &mut Vec<Effect>) {
        if started == self.last_started {
            return;
        }
        self.last_started = started;
        if let Some(id) = current {
            self.refresh(id, effects);
        }
    }

    /// The library has published what the domain had asked of it: now the things that
    /// depend on its state can be done (the Home's artists, the tracks without a cover).
    pub(crate) fn after_library(&mut self, library: &LibraryView, effects: &mut Vec<Effect>) {
        if std::mem::take(&mut self.startup) {
            let saved = library
                .liked
                .iter()
                .chain(library.recent.iter())
                .chain(library.playlists.iter().flat_map(|p| p.tracks.iter()));
            let mut ids: Vec<Uuid> = Vec::new();
            for track in saved {
                if track.cover_url.is_none() && !ids.contains(&track.id) {
                    ids.push(track.id);
                }
            }
            ids.truncate(STALE_REFRESH);
            for id in ids {
                self.refresh(id, effects);
            }
        }
        if std::mem::take(&mut self.prefetch_home) {
            // The artists the Home puts in view (followed and the catalog's first ones): they are loaded
            // at once, so opening one is immediate.
            let followed = library.followed.iter().take(4).map(|a| a.id);
            let shown = library.feed.artists.iter().take(6).map(|card| card.artist.id);
            let ids: Vec<Uuid> = followed.chain(shown).collect();
            self.prefetch(ids, effects);
        }
    }

    // ---------- Commands ----------

    pub(crate) fn apply(&mut self, command: CatalogCmd, now: Instant, effects: &mut Vec<Effect>) {
        match command {
            CatalogCmd::Ensure(Resource::Home) => {
                let home = &self.view.home;
                if home.recent.needs_load() || home.alphabetical.needs_load() || home.albums.needs_load() {
                    self.load_home(effects);
                } else {
                    // The plays have changed since last time: the mixes and the artists are made again.
                    self.library.push(LibraryCmd::RefreshFeed);
                }
            }
            CatalogCmd::Retry(Resource::Home) => self.load_home(effects),
            CatalogCmd::Ensure(Resource::Library) => {
                let listing = &self.view.library;
                let (tracks, albums) = (listing.tracks.needs_load(), listing.albums.needs_load());
                if tracks {
                    self.load_library_tracks(0, false, effects);
                }
                if albums {
                    self.load_library_albums(effects);
                }
            }
            CatalogCmd::Retry(Resource::Library) => {
                self.load_library_tracks(0, false, effects);
                self.load_library_albums(effects);
            }
            CatalogCmd::Ensure(Resource::Album(id)) => {
                if !matches!(&self.view.album, Some((open, load)) if *open == id && !load.needs_load()) {
                    self.open_album(id, false, now, effects);
                }
            }
            CatalogCmd::Retry(Resource::Album(id)) => self.open_album(id, true, now, effects),
            CatalogCmd::Ensure(Resource::Artist(id)) => {
                if !matches!(&self.view.artist, Some((open, load)) if *open == id && !load.needs_load()) {
                    self.open_artist(id, false, now, effects);
                }
            }
            CatalogCmd::Retry(Resource::Artist(id)) => self.open_artist(id, true, now, effects),
            CatalogCmd::LibraryMore => {
                let listing = &self.view.library;
                if let (Some(offset), false) = (listing.next_offset, listing.loading_more) {
                    self.view_mut().library.loading_more = true;
                    self.load_library_tracks(u32::try_from(offset).unwrap_or(0), true, effects);
                }
            }
            CatalogCmd::LibrarySort(sort) => {
                if self.view.library.sort != sort {
                    // The pages on the way with the earlier order are no longer right.
                    self.library_epoch += 1;
                    self.view_mut().library.sort = sort;
                    self.load_library_tracks(0, false, effects);
                    self.load_library_albums(effects);
                }
            }
            CatalogCmd::SearchInput(text) => self.search_input(text, effects),
            CatalogCmd::Search(text) => {
                self.ticket += 1;
                self.draft = text;
                effects.push(Effect::CancelTimer(TimerKey::Catalog(Timer::Search)));
                self.run_search(effects);
            }
            CatalogCmd::ArchiveLibraryOpen(library) => {
                let identifier = library.identifier.clone();
                self.view_mut().search.open_library =
                    Some(OpenLibrary { library, items: Load::Loading, next_page: None, loading_more: false });
                Self::run(effects, Job::ArchiveLibrary { api: self.api(), identifier, page: 1 });
            }
            CatalogCmd::ArchiveLibraryMore => {
                let Some(open) = &self.view.search.open_library else { return };
                let Some(page) = open.next_page.filter(|_| !open.loading_more) else { return };
                let identifier = open.library.identifier.clone();
                if let Some(open) = &mut self.view_mut().search.open_library {
                    open.loading_more = true;
                }
                Self::run(effects, Job::ArchiveLibrary { api: self.api(), identifier, page });
            }
            CatalogCmd::ArchiveLibraryClose => {
                if self.view.search.open_library.is_some() {
                    self.view_mut().search.open_library = None;
                }
            }
            CatalogCmd::OpenMetaArtist(id) => Self::run(effects, Job::MetaArtist { api: self.api(), id }),
            CatalogCmd::Changed => self.changed(),
        }
    }

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Search => self.run_search(effects),
        }
    }

    // ---------- Home and library ----------

    fn load_home(&mut self, effects: &mut Vec<Effect>) {
        let home = &self.view.home;
        if !(home.recent.is_loading() && home.alphabetical.is_loading() && home.albums.is_loading()) {
            self.view_mut().home =
                HomeView { recent: Load::Loading, alphabetical: Load::Loading, albums: Load::Loading };
        }
        let epoch = self.home_epoch;
        for section in [Section::Recent, Section::Alphabetical] {
            Self::run(effects, Job::Home { api: self.api(), epoch, section });
        }
        Self::run(effects, Job::HomeAlbums { api: self.api(), epoch });
    }

    fn load_library_tracks(&mut self, offset: u32, append: bool, effects: &mut Vec<Effect>) {
        if !append {
            let listing = &mut self.view_mut().library;
            listing.tracks = Load::Loading;
            listing.loading_more = false;
        }
        let (sort, epoch) = (self.view.library.sort, self.library_epoch);
        Self::run(effects, Job::LibraryTracks { api: self.api(), epoch, sort, offset, append });
    }

    fn load_library_albums(&mut self, effects: &mut Vec<Effect>) {
        self.view_mut().library.albums = Load::Loading;
        let (sort, epoch) = (self.view.library.sort, self.library_epoch);
        Self::run(effects, Job::LibraryAlbums { api: self.api(), epoch, sort });
    }

    /// The catalog has changed: Home and library reload at the next visit, the replies on the way
    /// are discarded and the library drops the feed it had built.
    fn changed(&mut self) {
        self.home_epoch += 1;
        self.library_epoch += 1;
        let view = &self.view;
        let untouched = is_idle(&view.home.recent)
            && is_idle(&view.home.alphabetical)
            && is_idle(&view.home.albums)
            && is_idle(&view.library.tracks)
            && is_idle(&view.library.albums)
            && view.library.next_offset.is_none()
            && !view.library.loading_more
            && view.library.more_error.is_none();
        if !untouched {
            let view = self.view_mut();
            view.home = HomeView::default();
            view.library = LibraryListing { sort: view.library.sort, ..LibraryListing::default() };
        }
        self.prefetch_home = false;
        self.library.push(LibraryCmd::HomeTracks(None));
    }

    // ---------- Albums and artists ----------

    fn open_album(&mut self, id: Uuid, force: bool, now: Instant, effects: &mut Vec<Effect>) {
        let cached = if force { None } else { self.albums.get(id, now) };
        let Some((album, stale)) = cached else {
            self.view_mut().album = Some((id, Load::Loading));
            Self::run(effects, Job::Album { api: self.api(), id, silent: false });
            return;
        };
        self.after_album(&album);
        self.view_mut().album = Some((id, Load::Ready(album)));
        if stale {
            Self::run(effects, Job::Album { api: self.api(), id, silent: true });
        }
    }

    /// What follows a shown album: "Riproduci" almost always starts from the first track, so its head
    /// is prepared; the saved album takes the cover of now, and the saved copies of its tracks too.
    fn after_album(&mut self, album: &Album) {
        if let Some(first) = album.tracks.first() {
            self.playback.push(PlaybackCmd::Warm(first.id));
        }
        self.library
            .push(LibraryCmd::SetSavedAlbumCover { id: album.id, cover_url: album.cover_url.clone() });
        self.patch_covers(&album.tracks);
    }

    fn open_artist(&mut self, id: Uuid, force: bool, now: Instant, effects: &mut Vec<Effect>) {
        let cached = if force { None } else { self.artists.get(id, now) };
        let Some((page, stale)) = cached else {
            self.view_mut().artist = Some((id, Load::Loading));
            Self::run(effects, Job::Artist { api: self.api(), id, fetch: Fetch::Open });
            return;
        };
        self.view_mut().artist = Some((id, Load::Ready(page)));
        if stale {
            self.refresh_artist(id, effects);
        }
    }

    /// Downloads again a page already in the cache without showing a loading.
    fn refresh_artist(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        if self.loading.insert(id) {
            Self::run(effects, Job::Artist { api: self.api(), id, fetch: Fetch::Background });
        }
    }

    /// Loads in advance the pages of these artists, if it does not already have them: not in the cache and not already
    /// on the way, at most [`PREFETCH_BATCH`], without repetitions.
    fn prefetch(&mut self, ids: impl IntoIterator<Item = Uuid>, effects: &mut Vec<Effect>) {
        let mut wanted = Vec::new();
        for id in ids {
            if wanted.len() == PREFETCH_BATCH {
                break;
            }
            if !self.artists.contains(id) && !wanted.contains(&id) && self.loading.insert(id) {
                wanted.push(id);
            }
        }
        for id in wanted {
            Self::run(effects, Job::Artist { api: self.api(), id, fetch: Fetch::Background });
        }
    }

    /// The covers the catalog has just sent back correct those saved (queue, recent,
    /// liked, playlists, plays): a track saved before the server stopped giving Archive's waves
    /// as cover would keep their address.
    fn patch_covers(&mut self, fresh: &[Track]) {
        if fresh.is_empty() {
            return;
        }
        let fresh: HashMap<Uuid, Option<String>> =
            fresh.iter().map(|t| (t.id, t.cover_url.clone())).collect();
        let fresh = Arc::new(fresh);
        self.library.push(LibraryCmd::PatchCovers(Arc::clone(&fresh)));
        self.playback.push(PlaybackCmd::PatchCovers(fresh));
    }

    // ---------- Search ----------

    fn search_input(&mut self, text: String, effects: &mut Vec<Effect>) {
        if text == self.draft {
            return;
        }
        self.draft = text;
        self.ticket += 1;
        if clean_query(&self.draft).is_some() {
            effects.push(Effect::Timer { key: TimerKey::Catalog(Timer::Search), after: SEARCH_DEBOUNCE });
            return;
        }
        effects.push(Effect::CancelTimer(TimerKey::Catalog(Timer::Search)));
        let search = &self.view.search;
        if !matches!(search.results, Load::Idle) || !search.submitted.is_empty() {
            let search = &mut self.view_mut().search;
            search.results = Load::Idle;
            search.submitted.clear();
            self.query = Some(String::new());
        }
    }

    /// Launches the search of the written text, in parallel in the catalog, on Internet Archive and in the
    /// official catalog (tracks, artists, albums and playlists with covers and photos).
    fn run_search(&mut self, effects: &mut Vec<Effect>) {
        let Some(query) = clean_query(&self.draft) else { return };
        let ticket = self.ticket;
        let search = &mut self.view_mut().search;
        search.submitted.clone_from(&query);
        search.results = Load::Loading;
        search.discover = Load::Loading;
        search.libraries = Arc::default();
        search.open_library = None;
        search.sources = Load::Loading;
        search.playlists = Load::Loading;
        search.artists = Arc::default();
        self.library.push(LibraryCmd::RememberSearch(query.clone()));
        self.query = Some(query.clone());
        let (a, b, c) = (self.api(), self.api(), self.api());
        Self::run(effects, Job::Search { api: a, ticket, query: query.clone() });
        Self::run(effects, Job::Discover { api: b, ticket, query: query.clone() });
        Self::run(effects, Job::Meta { api: c, ticket, query });
    }

    // ---------- Rereading of tracks ----------

    /// Rereads a track from the server, if it has not already done so in this session.
    fn refresh(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        if self.refreshed.contains(&id) {
            return;
        }
        self.refreshed.push_back(id);
        if self.refreshed.len() > REFRESHED {
            self.refreshed.pop_front();
        }
        Self::run(effects, Job::Refresh { api: self.api(), id });
    }

    fn raise(&mut self, event: CatalogEvent) {
        self.events += 1;
        let raised = CatalogRaised { seq: self.events, event };
        let events = Arc::make_mut(&mut self.view_mut().events);
        events.push_back(raised);
        while events.len() > EVENTS {
            events.pop_front();
        }
    }

    // ---------- Replies ----------

    pub(crate) fn done(&mut self, done: Done, now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Home { epoch, section, result } => {
                if epoch != self.home_epoch {
                    return;
                }
                let value = Load::of(result.map(|page| Arc::<[Track]>::from(page.items)));
                match section {
                    Section::Recent => {
                        // With tracks, the library builds the feed and the Home shows them
                        // together with it, in the same snapshot.
                        if let Load::Ready(tracks) = &value {
                            self.library.push(LibraryCmd::HomeTracks(Some(Arc::clone(tracks))));
                            self.patch_covers(tracks);
                            self.prefetch_home = true;
                        }
                        self.view_mut().home.recent = value;
                    }
                    Section::Alphabetical => self.view_mut().home.alphabetical = value,
                }
            }
            Done::HomeAlbums { epoch, result } => {
                if epoch == self.home_epoch {
                    self.view_mut().home.albums = Load::of(result.map(|page| Arc::from(page.items)));
                }
            }
            Done::Search { ticket, result } => {
                if ticket != self.ticket {
                    return;
                }
                if let Ok(results) = &result {
                    let ids: Vec<Uuid> = results.artists.iter().take(4).map(|a| a.id).collect();
                    self.patch_covers(&results.tracks);
                    self.prefetch(ids, effects);
                }
                self.view_mut().search.results = Load::of(result.map(|results| Arc::new(*results)));
            }
            Done::Discover { ticket, result } => {
                if ticket != self.ticket {
                    return;
                }
                let search = &mut self.view_mut().search;
                match result {
                    Ok(page) => {
                        search.discover = Load::Ready(Arc::from(page.items));
                        search.libraries = Arc::from(page.libraries);
                    }
                    Err(err) => {
                        search.discover = Load::Failed(err);
                        search.libraries = Arc::default();
                    }
                }
            }
            Done::Meta { ticket, result } => {
                if ticket != self.ticket {
                    return;
                }
                let search = &mut self.view_mut().search;
                match result {
                    Ok(found) => {
                        search.artists = Arc::from(found.artists);
                        search.sources = Load::Ready(Arc::from(found.tracks));
                        search.playlists = Load::Ready(Arc::from(found.lists));
                    }
                    Err(err) => {
                        search.artists = Arc::default();
                        search.sources = Load::Failed(err);
                        search.playlists = Load::Idle;
                    }
                }
            }
            Done::LibraryTracks { epoch, append, result } => {
                if epoch != self.library_epoch {
                    return;
                }
                let listing = &mut self.view_mut().library;
                listing.loading_more = false;
                match result {
                    Ok(page) => {
                        listing.next_offset = page.next_offset;
                        listing.more_error = None;
                        match &mut listing.tracks {
                            Load::Ready(tracks) if append => tracks.make_mut().extend(page.items),
                            other => *other = Load::Ready(page.items.into()),
                        }
                    }
                    Err(err) if append => listing.more_error = Some(err),
                    Err(err) => listing.tracks = Load::Failed(err),
                }
            }
            Done::LibraryAlbums { epoch, result } => {
                if epoch != self.library_epoch {
                    return;
                }
                let listing = &mut self.view_mut().library;
                listing.albums_truncated = result.as_ref().is_ok_and(|page| page.next_offset.is_some());
                listing.albums = Load::of(result.map(|page| Arc::from(page.items)));
            }
            Done::Album { id, silent, result } => self.album_arrived(id, silent, result, now),
            Done::Artist { id, fetch, result } => self.artist_arrived(id, fetch, result, now),
            Done::ArchiveLibrary { identifier, page, result } => {
                let Some(open) =
                    self.view.search.open_library.as_ref().filter(|o| o.library.identifier == identifier)
                else {
                    // A reply for a library already closed is ignored.
                    return;
                };
                let mut open = open.clone();
                let failed = open.receive(page, result).err();
                self.view_mut().search.open_library = Some(open);
                if let Some(err) = failed {
                    self.raise(CatalogEvent::LibraryPageFailed(err));
                }
            }
            Done::MetaArtist(result) => self.raise(match result {
                Ok(id) => CatalogEvent::MetaArtistOpened(id),
                Err(err) => CatalogEvent::MetaArtistFailed(err),
            }),
            Done::Healed { old, result } => match result {
                Ok(new) => {
                    // It takes the old one's place in the queue and among the saved, and the track starts again.
                    self.playback.push(PlaybackCmd::ReplaceTrack { old, new: new.clone(), restart: true });
                    self.library.push(LibraryCmd::ReplaceTrack { old, new: (*new).clone() });
                    self.raise(CatalogEvent::Replaced(Arc::new(*new)));
                }
                // The track was not found again: if it is the one that was wanted to hear, it stays in error.
                Err(_) => self.playback.push(PlaybackCmd::HealFailed(old)),
            },
            Done::Refreshed(result) => {
                if let Ok(fresh) = result {
                    let old = fresh.id;
                    self.playback.push(PlaybackCmd::ReplaceTrack { old, new: fresh.clone(), restart: false });
                    self.library.push(LibraryCmd::ReplaceTrack { old, new: (*fresh).clone() });
                    if fresh.cover_url.is_some() {
                        self.raise(CatalogEvent::Replaced(Arc::new(*fresh)));
                    }
                }
            }
        }
    }

    fn album_arrived(&mut self, id: Uuid, silent: bool, result: Result<Box<Album>, AppError>, now: Instant) {
        let showing = matches!(&self.view.album, Some((open, _)) if *open == id);
        match result {
            Ok(album) => {
                let album = self.albums.put(id, *album, now);
                if showing {
                    self.after_album(&album);
                    self.view_mut().album = Some((id, Load::Ready(album)));
                }
            }
            // A silent update that fails leaves the page as it is.
            Err(_) if silent => {}
            Err(err) if showing => self.view_mut().album = Some((id, Load::Failed(err))),
            Err(_) => {}
        }
    }

    fn artist_arrived(
        &mut self,
        id: Uuid,
        fetch: Fetch,
        result: Result<Box<ArtistPage>, AppError>,
        now: Instant,
    ) {
        self.loading.remove(&id);
        let open = matches!(&self.view.artist, Some((open, _)) if *open == id);
        match (fetch, result) {
            (Fetch::Open, Ok(page)) => {
                let page = self.artists.put(id, *page, now);
                if open {
                    self.view_mut().artist = Some((id, Load::Ready(Arc::clone(&page))));
                    // The followed artist's image follows the one the catalog has now.
                    self.library.push(LibraryCmd::SetFollowedCover {
                        id,
                        cover_url: portrait_url(&page).map(str::to_owned),
                        only_if_missing: false,
                    });
                    self.patch_covers(&page.tracks);
                }
            }
            (Fetch::Open, Err(err)) => {
                if open {
                    self.view_mut().artist = Some((id, Load::Failed(err)));
                }
            }
            (Fetch::Background, Ok(page)) => {
                // It goes into the cache, and if it is the one open it replaces it without the view jumping.
                let page = self.artists.put(id, *page, now);
                let replace =
                    matches!(&self.view.artist, Some((open, Load::Ready(_) | Load::Idle)) if *open == id);
                if replace {
                    self.view_mut().artist = Some((id, Load::Ready(Arc::clone(&page))));
                }
                self.patch_covers(&page.tracks);
                self.raise(CatalogEvent::Prefetched(page));
            }
            (Fetch::Background, Err(_)) => {}
        }
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Home { api, epoch, section } => {
                let (sort, limit) = match section {
                    Section::Recent => (Sort::Recent, HOME_RELEASES),
                    Section::Alphabetical => (Sort::Title, HOME_SECTION_SIZE),
                };
                let result = match api {
                    Ok(api) => api.tracks(TrackQuery { q: None, sort, limit, offset: 0 }).await,
                    Err(err) => Err(err),
                };
                Done::Home { epoch, section, result }
            }
            Self::HomeAlbums { api, epoch } => {
                let result = match api {
                    Ok(api) => api.albums(Sort::Recent, HOME_ALBUMS, 0).await,
                    Err(err) => Err(err),
                };
                Done::HomeAlbums { epoch, result }
            }
            Self::Search { api, ticket, query } => {
                let result = match api {
                    Ok(api) => api.search(&query).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Search { ticket, result }
            }
            Self::Discover { api, ticket, query } => {
                let result = match api {
                    Ok(api) => api.discover(&query).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Discover { ticket, result }
            }
            Self::Meta { api, ticket, query } => {
                let result = match api {
                    Ok(api) => api.meta_search(&query).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Meta { ticket, result }
            }
            Self::LibraryTracks { api, epoch, sort, offset, append } => {
                let result = match api {
                    Ok(api) => api.tracks(TrackQuery { q: None, sort, limit: LIBRARY_PAGE, offset }).await,
                    Err(err) => Err(err),
                };
                Done::LibraryTracks { epoch, append, result }
            }
            Self::LibraryAlbums { api, epoch, sort } => {
                let result = match api {
                    Ok(api) => api.albums(sort, LIBRARY_ALBUMS, 0).await,
                    Err(err) => Err(err),
                };
                Done::LibraryAlbums { epoch, result }
            }
            Self::Album { api, id, silent } => {
                let result = match api {
                    Ok(api) => api.album(id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Album { id, silent, result }
            }
            Self::Artist { api, id, fetch } => {
                let result = match api {
                    Ok(api) => api.artist(id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Artist { id, fetch, result }
            }
            Self::ArchiveLibrary { api, identifier, page } => {
                let result = match api {
                    Ok(api) => api.library(&identifier, page).await,
                    Err(err) => Err(err),
                };
                Done::ArchiveLibrary { identifier, page, result }
            }
            Self::MetaArtist { api, id } => {
                let result = match api {
                    Ok(api) => api.open_meta_artist(id).await,
                    Err(err) => Err(err),
                };
                Done::MetaArtist(result)
            }
            Self::Heal { api, track } => {
                let old = track.id;
                let result = match api {
                    Ok(api) => api::heal_track(api, (*track).clone()).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Healed { old, result }
            }
            Self::Refresh { api, id } => {
                let result = match api {
                    Ok(api) => api.track(id).await.map(Box::new),
                    Err(err) => Err(err),
                };
                Done::Refreshed(result)
            }
        }
    }

    /// If the job is lost, the outcome is a network error: the domain does not keep waiting (and a
    /// search or a list does not stay "loading" forever).
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Home { epoch, section, .. } => {
                Done::Home { epoch: *epoch, section: *section, result: lost() }
            }
            Self::HomeAlbums { epoch, .. } => Done::HomeAlbums { epoch: *epoch, result: lost() },
            Self::Search { ticket, .. } => Done::Search { ticket: *ticket, result: lost() },
            Self::Discover { ticket, .. } => Done::Discover { ticket: *ticket, result: lost() },
            Self::Meta { ticket, .. } => Done::Meta { ticket: *ticket, result: lost() },
            Self::LibraryTracks { epoch, append, .. } => {
                Done::LibraryTracks { epoch: *epoch, append: *append, result: lost() }
            }
            Self::LibraryAlbums { epoch, .. } => Done::LibraryAlbums { epoch: *epoch, result: lost() },
            Self::Album { id, silent, .. } => Done::Album { id: *id, silent: *silent, result: lost() },
            Self::Artist { id, fetch, .. } => Done::Artist { id: *id, fetch: *fetch, result: lost() },
            Self::ArchiveLibrary { identifier, page, .. } => {
                Done::ArchiveLibrary { identifier: identifier.clone(), page: *page, result: lost() }
            }
            Self::MetaArtist { .. } => Done::MetaArtist(lost()),
            Self::Heal { track, .. } => Done::Healed { old: track.id, result: lost() },
            Self::Refresh { .. } => Done::Refreshed(lost()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures::track;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn page_of<T>(items: Vec<T>, next_offset: Option<i64>) -> Page<T> {
        Page { items, limit: 50, offset: 0, next_offset }
    }

    fn artist(n: u128) -> ArtistPage {
        ArtistPage {
            id: id(n),
            name: format!("Artista {n}"),
            image_url: None,
            albums: Vec::new(),
            tracks: vec![track(n)],
        }
    }

    fn album(n: u128) -> Album {
        Album {
            id: id(n),
            title: format!("Album {n}"),
            released_on: None,
            artist: crate::api::models::ArtistRef { id: id(1000), name: "Artista".into() },
            cover_url: Some(format!("cover-{n}")),
            tracks: vec![track(n * 10), track(n * 10 + 1)],
        }
    }

    fn network() -> AppError {
        AppError::Network
    }

    /// The domain alone, with a fake clock that is moved by hand.
    struct Rig {
        catalog: Catalog,
        now: Instant,
    }

    impl Rig {
        fn new() -> Self {
            Self { catalog: Catalog::default(), now: Instant::now() }
        }

        fn cmd(&mut self, command: CatalogCmd) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.catalog.apply(command, self.now, &mut effects);
            effects
        }

        fn done(&mut self, done: Done) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.catalog.done(done, self.now, &mut effects);
            effects
        }

        fn fire(&mut self) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.catalog.fire(Timer::Search, self.now, &mut effects);
            effects
        }

        fn view(&self) -> Arc<CatalogView> {
            Arc::clone(&self.catalog.view)
        }

        fn wait(&mut self, by: Duration) {
            self.now += by;
        }
    }

    /// The jobs the effects ask for.
    fn jobs(effects: Vec<Effect>) -> Vec<Job> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Catalog(job)) => Some(job),
                _ => None,
            })
            .collect()
    }

    fn timers(effects: &[Effect]) -> Vec<Duration> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Timer { key: TimerKey::Catalog(Timer::Search), after } => Some(*after),
                _ => None,
            })
            .collect()
    }

    fn ticket_of(job: &Job) -> u64 {
        match job {
            Job::Search { ticket, .. } | Job::Discover { ticket, .. } | Job::Meta { ticket, .. } => *ticket,
            other => panic!("not a search job: {other:?}"),
        }
    }

    // ---------- Search ----------

    /// `typing_debounces_and_late_results_are_dropped`: each key re-arms the 300 ms wait, only the last
    /// search starts, and the replies of a superseded request are discarded.
    #[test]
    fn typing_debounces_and_late_results_are_dropped() {
        let mut rig = Rig::new();
        let first = rig.cmd(CatalogCmd::SearchInput("lo".into()));
        assert_eq!(timers(&first), [SEARCH_DEBOUNCE], "il primo tasto arma l'attesa");
        assert!(jobs(first).is_empty(), "mentre si scrive non parte nessuna ricerca");
        let second = rig.cmd(CatalogCmd::SearchInput("low".into()));
        assert_eq!(timers(&second), [SEARCH_DEBOUNCE], "il secondo la riarma: è un debounce");
        assert!(matches!(rig.view().search.results, Load::Idle));

        // The wait expires: three searches in parallel, all of the same request.
        let started = jobs(rig.fire());
        assert_eq!(started.len(), 3);
        let ticket = ticket_of(&started[0]);
        assert!(started.iter().all(|job| ticket_of(job) == ticket));
        assert!(matches!(&started[0], Job::Search { query, .. } if query == "low"));
        let view = rig.view();
        assert_eq!(view.search.submitted, "low");
        assert!(view.search.results.is_loading() && view.search.discover.is_loading());
        assert!(view.search.sources.is_loading() && view.search.playlists.is_loading());
        let library = rig.catalog.take_library();
        assert!(matches!(library.as_slice(), [LibraryCmd::RememberSearch(q)] if q == "low"), "{library:?}");
        assert_eq!(rig.catalog.take_query(), Some("low".into()));

        // The reply of a superseded request is discarded; the right one is shown.
        let found = |n| SearchResults { tracks: vec![track(n)], ..SearchResults::default() };
        let before = rig.view();
        rig.done(Done::Search { ticket: ticket - 1, result: Ok(Box::new(found(1))) });
        assert!(Arc::ptr_eq(&before, &rig.view()), "una risposta superata non cambia niente");
        rig.done(Done::Search { ticket, result: Ok(Box::new(found(2))) });
        assert_eq!(rig.view().search.results, Load::Ready(Arc::new(found(2))));

        // Writing again: a late reply does not overwrite the results.
        rig.cmd(CatalogCmd::SearchInput("lowe".into()));
        let before = rig.view();
        rig.done(Done::Search { ticket, result: Ok(Box::new(found(3))) });
        assert!(Arc::ptr_eq(&before, &rig.view()));
        assert_eq!(rig.view().search.results, Load::Ready(Arc::new(found(2))));
    }

    #[test]
    fn the_three_answers_fill_their_own_parts_and_a_failure_does_not_spill() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Search("rock".into()));
        let ticket = rig.catalog.ticket;
        rig.done(Done::Discover { ticket, result: Err(network()) });
        rig.done(Done::Meta {
            ticket,
            result: Ok(Box::new(MetaSearch { tracks: Vec::new(), artists: Vec::new(), lists: Vec::new() })),
        });
        let view = rig.view();
        assert_eq!(view.search.discover, Load::Failed(network()));
        assert!(view.search.libraries.is_empty());
        assert!(
            matches!(view.search.sources, Load::Ready(_)) && matches!(view.search.playlists, Load::Ready(_))
        );
        assert!(view.search.results.is_loading(), "the first search still waits for its own answer");
        rig.done(Done::Meta { ticket, result: Err(network()) });
        assert_eq!(rig.view().search.sources, Load::Failed(network()));
        assert_eq!(rig.view().search.playlists, Load::Idle);
    }

    #[test]
    fn clearing_the_text_clears_the_results_and_cancels_the_wait() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Search("low".into()));
        let ticket = rig.catalog.ticket;
        rig.done(Done::Search { ticket, result: Ok(Box::default()) });
        assert!(matches!(rig.view().search.results, Load::Ready(_)));
        rig.catalog.take_query();

        let effects = rig.cmd(CatalogCmd::SearchInput("  \u{7} ".into()));
        assert!(
            effects.iter().any(|e| matches!(e, Effect::CancelTimer(TimerKey::Catalog(_)))),
            "{effects:?}"
        );
        assert!(timers(&effects).is_empty());
        let view = rig.view();
        assert_eq!((&view.search.results, view.search.submitted.as_str()), (&Load::Idle, ""));
        assert_eq!(rig.catalog.take_query(), Some(String::new()), "the saved query follows");
        // And if there was nothing to delete, nothing changes.
        let before = rig.view();
        rig.cmd(CatalogCmd::SearchInput(String::new()));
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    /// Performance: the same text twice arms nothing and does not change the view, and the wait
    /// expired without a valid text starts nothing.
    #[test]
    fn an_identical_input_changes_nothing_and_an_empty_wait_starts_nothing() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::SearchInput("low".into()));
        let before = rig.view();
        let ticket = rig.catalog.ticket;
        let again = rig.cmd(CatalogCmd::SearchInput("low".into()));
        assert!(again.is_empty(), "{again:?}");
        assert_eq!(rig.catalog.ticket, ticket, "the request number does not move");
        assert!(Arc::ptr_eq(&before, &rig.view()));

        let mut rig = Rig::new();
        assert!(rig.fire().is_empty(), "nothing typed: nothing to search");
    }

    #[test]
    fn a_search_answer_prefetches_the_first_artists_and_corrects_the_saved_covers() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Search("rock".into()));
        let ticket = rig.catalog.ticket;
        let artists: Vec<_> = (1..=6)
            .map(|n| crate::api::ArtistSummary {
                id: id(n),
                name: format!("A{n}"),
                track_count: 1,
                album_count: 1,
                cover_url: None,
            })
            .collect();
        let results = SearchResults { tracks: vec![track(1)], artists, ..SearchResults::default() };
        let effects = rig.done(Done::Search { ticket, result: Ok(Box::new(results)) });
        let prefetched: Vec<Uuid> = jobs(effects)
            .into_iter()
            .filter_map(|job| match job {
                Job::Artist { id, fetch: Fetch::Background, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(prefetched, [id(1), id(2), id(3), id(4)], "the first four");
        assert!(rig.catalog.take_library().iter().any(|c| matches!(c, LibraryCmd::PatchCovers(_))));
        assert!(rig.catalog.take_playback().iter().any(|c| matches!(c, PlaybackCmd::PatchCovers(_))));
    }

    // ---------- Artists and albums ----------

    #[test]
    fn the_artist_cache_forgets_the_least_used_page() {
        let mut cache: Lru<ArtistPage> = Lru::new(ARTIST_PAGES);
        let now = Instant::now();
        for n in 0..ARTIST_PAGES as u128 {
            cache.put(id(n), artist(n), now);
        }
        // Rereading the oldest one saves it: the next one goes out.
        assert!(cache.get(id(0), now).is_some());
        cache.put(id(1000), artist(1000), now);
        assert_eq!(cache.len(), ARTIST_PAGES, "the capacity is fixed");
        assert!(cache.contains(id(0)) && !cache.contains(id(1)));
        // Putting one already present back replaces it without making the cache grow.
        cache.put(id(0), artist(0), now);
        assert_eq!(cache.len(), ARTIST_PAGES);
    }

    #[test]
    fn a_cached_page_is_fresh_then_stale() {
        let mut cache: Lru<ArtistPage> = Lru::new(4);
        let now = Instant::now();
        cache.put(id(1), artist(1), now);
        assert_eq!(cache.get(id(1), now).map(|(_, stale)| stale), Some(false));
        let later = now + STALE_AFTER + Duration::from_secs(1);
        assert_eq!(cache.get(id(1), later).map(|(_, stale)| stale), Some(true));
        assert!(cache.get(id(2), now).is_none());
    }

    #[test]
    fn an_opened_artist_loads_then_opens_at_once_from_the_cache_and_refreshes_when_stale() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7)))));
        assert!(matches!(started.as_slice(), [Job::Artist { fetch: Fetch::Open, .. }]));
        assert!(matches!(rig.view().artist, Some((open, Load::Loading)) if open == id(7)));
        // Already on the way: opening it again asks for nothing.
        assert!(jobs(rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7))))).is_empty());

        rig.done(Done::Artist { id: id(7), fetch: Fetch::Open, result: Ok(Box::new(artist(7))) });
        let view = rig.view();
        let Some((_, Load::Ready(page))) = &view.artist else { panic!("{:?}", view.artist) };
        assert_eq!(page.id, id(7));
        // A followed artist takes the catalog's image and the saved covers are corrected.
        let library = rig.catalog.take_library();
        assert!(library.iter().any(
            |c| matches!(c, LibraryCmd::SetFollowedCover { id: i, only_if_missing: false, .. } if *i == id(7))
        ));
        assert!(library.iter().any(|c| matches!(c, LibraryCmd::PatchCovers(_))));

        // Elsewhere, then back here: from the cache, with no "loading" and no network (it is fresh).
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(8))));
        let effects = rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7))));
        assert!(jobs(effects).is_empty(), "fresh: no request");
        assert!(matches!(&rig.view().artist, Some((open, Load::Ready(_))) if *open == id(7)));

        // After 90 seconds it is shown at once and reread silently.
        rig.wait(STALE_AFTER + Duration::from_secs(1));
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(8))));
        let refreshed = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7)))));
        assert!(matches!(refreshed.as_slice(), [Job::Artist { fetch: Fetch::Background, .. }]));
        assert!(
            matches!(&rig.view().artist, Some((open, Load::Ready(_))) if *open == id(7)),
            "shown at once"
        );
        // The silent reply replaces the open page without the view jumping.
        let mut fresher = artist(7);
        fresher.name = "Nuovo nome".into();
        rig.done(Done::Artist { id: id(7), fetch: Fetch::Background, result: Ok(Box::new(fresher)) });
        let view = rig.view();
        assert!(matches!(&view.artist, Some((_, Load::Ready(page))) if page.name == "Nuovo nome"));
        assert!(
            matches!(view.events.back().map(|e| &e.event), Some(CatalogEvent::Prefetched(p)) if p.id == id(7))
        );
    }

    #[test]
    fn retrying_an_artist_ignores_the_cache_and_a_failure_is_shown() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(7))));
        rig.done(Done::Artist { id: id(7), fetch: Fetch::Open, result: Ok(Box::new(artist(7))) });
        let started = jobs(rig.cmd(CatalogCmd::Retry(Resource::Artist(id(7)))));
        assert!(matches!(started.as_slice(), [Job::Artist { fetch: Fetch::Open, .. }]));
        assert!(matches!(rig.view().artist, Some((_, Load::Loading))), "retry shows the loading state");
        rig.done(Done::Artist { id: id(7), fetch: Fetch::Open, result: Err(network()) });
        assert!(matches!(&rig.view().artist, Some((_, Load::Failed(e))) if *e == network()));
        // A page that arrives for an artist no longer open does not touch the view.
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(9))));
        let before = rig.view();
        rig.done(Done::Artist { id: id(7), fetch: Fetch::Open, result: Ok(Box::new(artist(7))) });
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn prefetching_skips_known_and_pending_artists_and_is_capped_per_batch() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(1))));
        rig.done(Done::Artist { id: id(1), fetch: Fetch::Open, result: Ok(Box::new(artist(1))) });
        let mut effects = Vec::new();
        rig.catalog.prefetch([1, 2, 2, 3].map(id), &mut effects);
        let asked: Vec<Uuid> = jobs(effects)
            .into_iter()
            .filter_map(|job| match job {
                Job::Artist { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(asked, [id(2), id(3)]);
        // Already on the way: no second request; arrived, in the cache: not even that.
        let mut effects = Vec::new();
        rig.catalog.prefetch([id(2)], &mut effects);
        assert!(effects.is_empty());
        rig.done(Done::Artist { id: id(2), fetch: Fetch::Background, result: Ok(Box::new(artist(2))) });
        rig.catalog.prefetch([id(2)], &mut effects);
        assert!(effects.is_empty(), "arrived: it is in the cache");

        let mut effects = Vec::new();
        rig.catalog.prefetch((100..150).map(id), &mut effects);
        assert_eq!(jobs(effects).len(), PREFETCH_BATCH);
    }

    #[test]
    fn an_album_opens_from_the_cache_and_warms_its_first_track() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Album(id(5)))));
        assert!(matches!(started.as_slice(), [Job::Album { silent: false, .. }]));
        rig.done(Done::Album { id: id(5), silent: false, result: Ok(Box::new(album(5))) });
        assert!(
            matches!(&rig.view().album, Some((open, Load::Ready(a))) if *open == id(5) && a.tracks.len() == 2)
        );
        // The head of the first track, the saved album's cover and those of the tracks.
        let playback = rig.catalog.take_playback();
        assert!(
            matches!(playback.as_slice(), [PlaybackCmd::Warm(first), PlaybackCmd::PatchCovers(_)] if *first == id(50)),
            "{playback:?}"
        );
        let library = rig.catalog.take_library();
        assert!(matches!(library.first(), Some(LibraryCmd::SetSavedAlbumCover { id: a, .. }) if *a == id(5)));

        // Coming back after another album, from the cache: no request.
        rig.cmd(CatalogCmd::Ensure(Resource::Album(id(6))));
        assert!(jobs(rig.cmd(CatalogCmd::Ensure(Resource::Album(id(5))))).is_empty());
        assert!(matches!(&rig.view().album, Some((open, Load::Ready(_))) if *open == id(5)));
        // Old: it is shown at once and reread silently, and if the update fails it stays.
        rig.wait(STALE_AFTER + Duration::from_secs(1));
        rig.cmd(CatalogCmd::Ensure(Resource::Album(id(6))));
        let silent = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Album(id(5)))));
        assert!(matches!(silent.as_slice(), [Job::Album { silent: true, .. }]));
        let before = rig.view();
        rig.done(Done::Album { id: id(5), silent: true, result: Err(network()) });
        assert!(Arc::ptr_eq(&before, &rig.view()), "a failed silent refresh leaves the page as it is");
        // "Riprova" throws away the cache and shows the loading.
        assert!(matches!(
            jobs(rig.cmd(CatalogCmd::Retry(Resource::Album(id(5))))).as_slice(),
            [Job::Album { silent: false, .. }]
        ));
        assert!(matches!(rig.view().album, Some((_, Load::Loading))));
        rig.done(Done::Album { id: id(5), silent: false, result: Err(network()) });
        assert!(matches!(&rig.view().album, Some((_, Load::Failed(_)))));
    }

    #[test]
    fn the_album_cache_is_bounded() {
        let mut cache: Lru<Album> = Lru::new(ALBUM_PAGES);
        let now = Instant::now();
        for n in 0..(ALBUM_PAGES as u128 * 3) {
            cache.put(id(n), album(n), now);
        }
        assert_eq!(cache.len(), ALBUM_PAGES);
        assert!(!cache.contains(id(0)) && cache.contains(id(ALBUM_PAGES as u128 * 3 - 1)));
    }

    // ---------- Home ----------

    fn home_answers(rig: &mut Rig, epoch: u64) {
        rig.done(Done::Home {
            epoch,
            section: Section::Recent,
            result: Ok(page_of((1..=4).map(track).collect(), None)),
        });
        rig.done(Done::Home {
            epoch,
            section: Section::Alphabetical,
            result: Ok(page_of(vec![track(9)], None)),
        });
        rig.done(Done::HomeAlbums { epoch, result: Ok(page_of(Vec::new(), None)) });
    }

    #[test]
    fn the_home_loads_three_shelves_and_hands_the_recent_tracks_to_the_library() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Home)));
        assert_eq!(started.len(), 3);
        let view = rig.view();
        assert!(
            view.home.recent.is_loading()
                && view.home.alphabetical.is_loading()
                && view.home.albums.is_loading()
        );
        // While loading it is not asked again.
        assert!(rig.cmd(CatalogCmd::Ensure(Resource::Home)).iter().all(|e| !matches!(e, Effect::Run(_))));
        rig.catalog.take_library();

        let epoch = rig.catalog.home_epoch();
        home_answers(&mut rig, epoch);
        let view = rig.view();
        assert!(matches!(&view.home.recent, Load::Ready(t) if t.len() == 4));
        let library = rig.catalog.take_library();
        let [LibraryCmd::HomeTracks(Some(tracks)), LibraryCmd::PatchCovers(_)] = library.as_slice() else {
            panic!("{library:?}");
        };
        let Load::Ready(shown) = &view.home.recent else { panic!() };
        assert!(Arc::ptr_eq(tracks, shown), "la libreria costruisce il feed dagli stessi brani");

        // Ready: coming back only asks to make the feed again, if needed.
        let effects = rig.cmd(CatalogCmd::Ensure(Resource::Home));
        assert!(effects.is_empty());
        assert!(matches!(rig.catalog.take_library().as_slice(), [LibraryCmd::RefreshFeed]));
        // An error is retried.
        rig.done(Done::HomeAlbums { epoch, result: Err(network()) });
        assert!(jobs(rig.cmd(CatalogCmd::Ensure(Resource::Home))).len() == 3);
    }

    #[test]
    fn a_changed_catalogue_resets_the_home_and_the_listing_and_drops_late_answers() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Ensure(Resource::Home));
        rig.cmd(CatalogCmd::Ensure(Resource::Library));
        let old = rig.catalog.home_epoch();
        home_answers(&mut rig, old);
        rig.catalog.take_library();

        rig.cmd(CatalogCmd::Changed);
        let view = rig.view();
        assert!(view.home.recent.needs_load() && matches!(view.home.recent, Load::Idle));
        assert!(matches!(view.library.tracks, Load::Idle) && matches!(view.library.albums, Load::Idle));
        let library = rig.catalog.take_library();
        assert!(matches!(library.as_slice(), [LibraryCmd::HomeTracks(None)]), "{library:?}");

        // A reply from before the change does not come back to life.
        let before = rig.view();
        home_answers(&mut rig, old);
        rig.done(Done::LibraryTracks { epoch: 0, append: false, result: Ok(page_of(vec![track(1)], None)) });
        assert!(Arc::ptr_eq(&before, &rig.view()));
        assert!(rig.catalog.take_library().is_empty());
        // Changing an already empty catalog publishes nothing.
        rig.cmd(CatalogCmd::Changed);
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn a_new_server_drops_the_caches_and_reloads_the_home() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::Ensure(Resource::Artist(id(1))));
        rig.done(Done::Artist { id: id(1), fetch: Fetch::Open, result: Ok(Box::new(artist(1))) });
        let mut effects = Vec::new();
        rig.catalog.set_server(&None, &mut effects);
        assert!(effects.is_empty(), "the same server: nothing");
        rig.catalog.set_server(&Some("http://127.0.0.1:9".into()), &mut effects);
        assert_eq!(jobs(effects).len(), 3, "the home loads again");
        assert!(!rig.catalog.artists.contains(id(1)));
        assert!(rig.view().artist.is_none());
    }

    // ---------- Library ----------

    #[test]
    fn the_listing_appends_pages_and_a_new_order_drops_the_old_pages() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Ensure(Resource::Library)));
        assert!(matches!(
            started.as_slice(),
            [Job::LibraryTracks { offset: 0, append: false, .. }, Job::LibraryAlbums { .. }]
        ));
        let epoch = rig.catalog.library_epoch();
        rig.done(Done::LibraryTracks {
            epoch,
            append: false,
            result: Ok(page_of(vec![track(1), track(2)], Some(2))),
        });
        let view = rig.view();
        assert_eq!(view.library.next_offset, Some(2));
        assert!(matches!(&view.library.tracks, Load::Ready(t) if t.len() == 2));

        // "Altri": one page at a time, and not two requests together.
        let more = jobs(rig.cmd(CatalogCmd::LibraryMore));
        assert!(matches!(more.as_slice(), [Job::LibraryTracks { offset: 2, append: true, .. }]));
        assert!(rig.view().library.loading_more);
        assert!(jobs(rig.cmd(CatalogCmd::LibraryMore)).is_empty());
        rig.done(Done::LibraryTracks { epoch, append: true, result: Ok(page_of(vec![track(3)], None)) });
        let view = rig.view();
        assert!(matches!(&view.library.tracks, Load::Ready(t) if t.len() == 3));
        assert!(!view.library.loading_more && view.library.next_offset.is_none());

        // A next page that fails leaves the tracks and says the error.
        rig.done(Done::LibraryTracks { epoch, append: true, result: Err(network()) });
        assert_eq!(rig.view().library.more_error, Some(network()));
        assert!(matches!(&rig.view().library.tracks, Load::Ready(t) if t.len() == 3));

        // Another order: it reloads everything and the pages on the way with the earlier order are discarded.
        let same = rig.cmd(CatalogCmd::LibrarySort(Sort::Recent));
        assert!(same.is_empty(), "the same order changes nothing");
        let sorted = jobs(rig.cmd(CatalogCmd::LibrarySort(Sort::Title)));
        assert!(matches!(
            sorted.as_slice(),
            [
                Job::LibraryTracks { sort: Sort::Title, offset: 0, .. },
                Job::LibraryAlbums { sort: Sort::Title, .. }
            ]
        ));
        assert!(rig.view().library.tracks.is_loading() && rig.view().library.albums.is_loading());
        let before = rig.view();
        rig.done(Done::LibraryTracks { epoch, append: true, result: Ok(page_of(vec![track(4)], None)) });
        assert!(Arc::ptr_eq(&before, &rig.view()), "an old page does not land in the new order");
        let epoch = rig.catalog.library_epoch();
        rig.done(Done::LibraryAlbums { epoch, result: Ok(page_of(Vec::new(), Some(100))) });
        assert!(rig.view().library.albums_truncated);
    }

    // ---------- Archive libraries and the official catalog ----------

    fn library(identifier: &str) -> DiscoverLibrary {
        DiscoverLibrary {
            identifier: identifier.into(),
            title: "Netlabel".into(),
            creator: None,
            found: 2,
            cover_url: None,
        }
    }

    fn item(identifier: &str) -> DiscoverItem {
        DiscoverItem {
            identifier: identifier.into(),
            title: identifier.into(),
            creator: None,
            licence: "cc-by".into(),
            year: None,
            cover_url: None,
            lossless: true,
        }
    }

    fn items(page: u32, next_page: Option<u32>, names: &[&str]) -> Result<LibraryPage, AppError> {
        let _ = page;
        Ok(LibraryPage { items: names.iter().map(|n| item(n)).collect(), next_page })
    }

    #[test]
    fn library_pages_are_appended_in_order_and_a_failure_is_an_event() {
        let mut rig = Rig::new();
        let first = jobs(rig.cmd(CatalogCmd::ArchiveLibraryOpen(library("net"))));
        assert!(matches!(first.as_slice(), [Job::ArchiveLibrary { page: 1, .. }]));
        assert!(matches!(&rig.view().search.open_library, Some(o) if o.items.is_loading()));
        rig.done(Done::ArchiveLibrary {
            identifier: "net".into(),
            page: 1,
            result: items(1, Some(2), &["a", "b"]),
        });
        let more = jobs(rig.cmd(CatalogCmd::ArchiveLibraryMore));
        assert!(matches!(more.as_slice(), [Job::ArchiveLibrary { page: 2, .. }]));
        assert!(jobs(rig.cmd(CatalogCmd::ArchiveLibraryMore)).is_empty(), "una pagina alla volta");
        rig.done(Done::ArchiveLibrary { identifier: "net".into(), page: 2, result: items(2, None, &["c"]) });
        let open = rig.view().search.open_library.clone().expect("open");
        let Load::Ready(all) = &open.items else { panic!() };
        assert_eq!(all.iter().map(|i| i.identifier.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!((open.next_page, open.loading_more), (None, false));

        // A next page that fails: the albums stay and it can be retried.
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::ArchiveLibraryOpen(library("net")));
        rig.done(Done::ArchiveLibrary {
            identifier: "net".into(),
            page: 1,
            result: items(1, Some(2), &["a"]),
        });
        rig.cmd(CatalogCmd::ArchiveLibraryMore);
        rig.done(Done::ArchiveLibrary { identifier: "net".into(), page: 2, result: Err(network()) });
        let open = rig.view().search.open_library.clone().expect("open");
        assert!(matches!(&open.items, Load::Ready(i) if i.len() == 1));
        assert_eq!((open.next_page, open.loading_more), (Some(2), false));
        assert!(matches!(
            rig.view().events.back().map(|e| &e.event),
            Some(CatalogEvent::LibraryPageFailed(_))
        ));

        // A first page that fails takes the place of the albums.
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::ArchiveLibraryOpen(library("net")));
        rig.done(Done::ArchiveLibrary { identifier: "net".into(), page: 1, result: Err(network()) });
        assert!(matches!(&rig.view().search.open_library, Some(o) if matches!(o.items, Load::Failed(_))));

        // Closed, a late reply is ignored.
        rig.cmd(CatalogCmd::ArchiveLibraryClose);
        let before = rig.view();
        rig.done(Done::ArchiveLibrary { identifier: "net".into(), page: 1, result: items(1, None, &["x"]) });
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn a_new_search_closes_the_open_library() {
        let mut rig = Rig::new();
        rig.cmd(CatalogCmd::ArchiveLibraryOpen(library("net")));
        rig.cmd(CatalogCmd::Search("altro".into()));
        assert!(rig.view().search.open_library.is_none());
    }

    #[test]
    fn opening_a_catalogue_artist_raises_an_event_with_the_new_id_or_the_error() {
        let mut rig = Rig::new();
        assert!(matches!(
            jobs(rig.cmd(CatalogCmd::OpenMetaArtist(77))).as_slice(),
            [Job::MetaArtist { id: 77, .. }]
        ));
        rig.done(Done::MetaArtist(Ok(id(5))));
        rig.done(Done::MetaArtist(Err(network())));
        let events: Vec<_> = rig.view().events.iter().map(|e| (e.seq, e.event.clone())).collect();
        assert_eq!(
            events,
            [(1, CatalogEvent::MetaArtistOpened(id(5))), (2, CatalogEvent::MetaArtistFailed(network()))]
        );
    }

    #[test]
    fn the_event_list_is_bounded_and_numbers_never_repeat() {
        let mut rig = Rig::new();
        for n in 0..(EVENTS as u128 + 10) {
            rig.done(Done::MetaArtist(Ok(id(n))));
        }
        let view = rig.view();
        assert_eq!(view.events.len(), EVENTS);
        assert_eq!(view.events.back().map(|e| e.seq), Some(EVENTS as u64 + 10));
        assert_eq!(view.events.front().map(|e| e.seq), Some(11));
    }

    // ---------- Vanished and reread tracks ----------

    #[test]
    fn a_missing_track_is_healed_and_replaces_the_saved_copies() {
        let mut rig = Rig::new();
        let missing = track(1);
        let mut effects = Vec::new();
        rig.catalog.heal(Arc::new(missing.clone()), &mut effects);
        assert!(matches!(jobs(effects).as_slice(), [Job::Heal { track, .. }] if track.id == missing.id));

        let found = track(9);
        rig.done(Done::Healed { old: missing.id, result: Ok(Box::new(found.clone())) });
        let playback = rig.catalog.take_playback();
        assert!(
            matches!(playback.as_slice(), [PlaybackCmd::ReplaceTrack { old, new, restart: true }] if *old == missing.id && new.id == found.id),
            "{playback:?}"
        );
        let library = rig.catalog.take_library();
        assert!(
            matches!(library.as_slice(), [LibraryCmd::ReplaceTrack { old, new }] if *old == missing.id && new.id == found.id)
        );
        assert!(
            matches!(rig.view().events.back().map(|e| &e.event), Some(CatalogEvent::Replaced(t)) if t.id == found.id)
        );

        // A track that is not found again: playback stays in error.
        rig.done(Done::Healed { old: missing.id, result: Err(AppError::InvalidResponse) });
        assert!(
            matches!(rig.catalog.take_playback().as_slice(), [PlaybackCmd::HealFailed(old)] if *old == missing.id)
        );
        assert!(rig.catalog.take_library().is_empty());
    }

    #[test]
    fn a_track_is_reread_once_per_session_and_its_cover_is_announced() {
        let mut rig = Rig::new();
        let mut effects = Vec::new();
        rig.catalog.track_started(1, Some(id(4)), &mut effects);
        rig.catalog.track_started(1, Some(id(4)), &mut effects);
        assert_eq!(jobs(effects).len(), 1, "the same start twice");
        let mut effects = Vec::new();
        rig.catalog.track_started(2, Some(id(4)), &mut effects);
        assert!(effects.is_empty(), "the same track, played again: not reread");
        rig.catalog.track_started(3, Some(id(5)), &mut effects);
        assert_eq!(jobs(effects).len(), 1);

        let mut fresh = track(4);
        fresh.cover_url = Some("nuova".into());
        rig.done(Done::Refreshed(Ok(Box::new(fresh.clone()))));
        let playback = rig.catalog.take_playback();
        assert!(matches!(playback.as_slice(), [PlaybackCmd::ReplaceTrack { restart: false, .. }]));
        assert!(matches!(rig.catalog.take_library().as_slice(), [LibraryCmd::ReplaceTrack { .. }]));
        assert!(
            matches!(rig.view().events.back().map(|e| &e.event), Some(CatalogEvent::Replaced(t)) if t.cover_url.as_deref() == Some("nuova"))
        );
        // An error does nothing.
        let before = rig.view();
        rig.done(Done::Refreshed(Err(network())));
        assert!(Arc::ptr_eq(&before, &rig.view()));
    }

    #[test]
    fn at_start_the_saved_tracks_without_a_cover_are_reread_up_to_the_limit() {
        use crate::config::CoreConfig;
        let mut rig = Rig::new();
        let mut config = CoreConfig::default();
        let mut with_cover = track(500);
        with_cover.cover_url = Some("c".into());
        config.liked = std::iter::once(with_cover).chain((1..=40).map(track)).collect::<Vec<_>>().into();
        let library = crate::state::library::Library::new(config.library(), 1);
        let mut effects = Vec::new();
        rig.catalog.after_library(&library.view, &mut effects);
        assert!(effects.is_empty(), "not before the start");
        rig.catalog.start();
        rig.catalog.after_library(&library.view, &mut effects);
        let ids: Vec<Uuid> = jobs(effects)
            .into_iter()
            .filter_map(|job| match job {
                Job::Refresh { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), STALE_REFRESH);
        assert!(!ids.contains(&id(500)), "the one with a cover is fine");
        let mut again = Vec::new();
        rig.catalog.after_library(&library.view, &mut again);
        assert!(again.is_empty(), "once");
    }

    #[test]
    fn a_lost_job_never_leaves_a_search_loading() {
        let mut rig = Rig::new();
        let started = jobs(rig.cmd(CatalogCmd::Search("rock".into())));
        for job in &started {
            let (Done::Search { .. } | Done::Discover { .. } | Done::Meta { .. }) = job.lost() else {
                panic!("{job:?}");
            };
        }
        for job in started {
            rig.done(job.lost());
        }
        let view = rig.view();
        assert!(
            matches!(view.search.results, Load::Failed(_)) && matches!(view.search.discover, Load::Failed(_))
        );
        assert!(matches!(view.search.sources, Load::Failed(_)));
    }
}
