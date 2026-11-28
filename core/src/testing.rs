//! A fake core for frontend tests: the same reducer as the real core, but on the test's thread.
//! A command applies at once and returns the snapshot the core would have published,
//! without threads, channels or runtime. The effects that do not touch the outside world run here:
//! the audio (a detached output, from which the test reads the samples) and the tick, which the test
//! fires with [`LocalCore::tick`]; the network jobs are collected to say what was asked
//! ([`LocalCore::take_jobs`]) and the test hands their outcome back by hand. Nothing writes to disk.
//!
//! It serves the tests that drive a frontend with a sequence of messages and look at its state (the
//! `config.json` and the rest of the real core are tested with `Core::start`). A real build does not
//! contain it: only the `test-support` feature turns it on.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use uuid::Uuid;

use std::path::PathBuf;

use crate::api::social::{Friends, JamInvite, MyProfile, Profile, PublicPlaylist, Relation, UserPage};
use crate::api::update::Offer;
use crate::api::{
    Album, AlbumSummary, ArtistPage, DiscoverImport, DiscoverPage, Downloaded, LibraryPage, MetaArtistPage,
    MetaSearch, Page, Progress, Quality, SearchResults, SourceKind, SourcePlaylistPage, Track,
};
use crate::command::Command;
use crate::config::Config;
use crate::effect::{self, Done, Effect, TimerKey};
use crate::error::AppError;
use crate::media;
use crate::offline::Lost;
use crate::player::AudioOutput;
use crate::player::lyrics::Text;
use crate::player::stream::StreamBuffer;
use crate::snapshot::Snapshot;
pub use crate::state::catalog::Section;
use crate::state::catalog::{self, Fetch};
use crate::state::downloads;
pub use crate::state::downloads::UpdateCheck;
use crate::state::external::{self, ImportTarget, Importing};
use crate::state::jam;
pub use crate::state::jam::JamOp;
use crate::state::playback::{self, run_audio};
use crate::state::social;
pub use crate::state::system::{DiscordNow, DiscordOp, MediaOp};
use crate::state::{CoreState, Input};

#[cfg(test)]
pub(crate) mod fake_backend;
#[cfg(test)]
pub(crate) mod fake_room;

/// A network job the core has asked for and the test has not resolved yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingJob {
    /// Open the track `track` for the load `generation`.
    Load {
        track: Uuid,
        generation: u64,
    },
    /// Prepare a track: its head, or the whole (`whole`).
    Warm {
        track: Uuid,
        whole: bool,
    },
    /// Other tracks of the artist, for automatic playback.
    Autoplay {
        artist: Uuid,
    },
    /// The text of the track `track`.
    Lyrics {
        track: Uuid,
    },
    /// The tracks of album `id`; `ticket` is the request number, to return with the reply.
    Album {
        id: Uuid,
        ticket: u64,
    },
    /// The artist of a radio started from `start`.
    Radio {
        start: Uuid,
        artist: Uuid,
        ticket: u64,
    },
    /// A Home shelf (recent or alphabetical).
    Home(Section),
    /// The Home's albums.
    HomeAlbums,
    /// The three searches of the text `query`, each with its request number.
    Search {
        ticket: u64,
        query: String,
    },
    Discover {
        ticket: u64,
        query: String,
    },
    Meta {
        ticket: u64,
        query: String,
    },
    /// A page of library tracks, from track `offset`.
    LibraryTracks {
        offset: u32,
        append: bool,
    },
    LibraryAlbums,
    /// An album to show; `silent` if it is a silent update.
    CatalogAlbum {
        id: Uuid,
        silent: bool,
    },
    /// An artist page; `background` if it is early or a silent update.
    CatalogArtist {
        id: Uuid,
        background: bool,
    },
    /// A page of albums of an Archive library.
    ArchiveLibrary {
        identifier: String,
        page: u32,
    },
    /// A job of the account, the profile, the friends or the presence.
    Social(SocialWork),
    /// The official catalog's artist to bring into the server.
    MetaArtist {
        id: u64,
    },
    /// A missing track to find again in the catalog.
    Heal {
        track: Uuid,
    },
    /// A track to reread from the server.
    Refresh {
        id: Uuid,
    },
    /// A track to download.
    Download {
        track: Uuid,
    },
    /// A file of a cancelled download to remove from disk.
    Discard,
    /// The latest release to read.
    CheckUpdate {
        kind: UpdateCheck,
    },
    /// The update package to download.
    DownloadUpdate,
    /// The downloaded files to check (`deep`: their content too).
    Verify {
        deep: bool,
    },
    /// The file of track `title`, removed from the index, to remove from disk.
    Remove {
        title: String,
    },
    /// The downloads folder to open.
    OpenFolder,
    /// The update package's folder to open.
    RevealUpdate,
    /// The tracks of an external playlist; `ticket` is the opening's number, to return.
    ExternalPlaylist {
        source: SourceKind,
        id: String,
        ticket: u64,
    },
    /// A track (or an Archive item) to bring into the catalog, and where it will go.
    Import {
        target: ImportTarget,
        key: Importing,
        video: bool,
    },
    /// An artist's popular tracks and discography from the official catalog.
    ArtistExtras {
        id: Uuid,
        name: String,
    },
    /// The catalog tracks that the jam's shared queue has and the local one does not.
    JamTracks {
        ids: Vec<Uuid>,
    },
    /// The address of the music video of `track`, for the request `ticket`.
    #[cfg(feature = "video")]
    VideoUrl {
        track: Uuid,
        ticket: u64,
    },
    /// The server must prepare the music video of `track`.
    #[cfg(feature = "video")]
    VideoPrepare {
        track: Uuid,
    },
}

/// A job of the account or the profile that the core has asked for: enough for a test to say
/// what has started (never the secrets: passwords, codes and tokens are not there).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocialWork {
    SignIn,
    SignUp,
    SubmitCode,
    ResendCode,
    /// The browser sign-in: the core puts the loopback in listening and asks Clerk for the provider.
    OauthBegin(crate::auth::Provider),
    /// The wait for the browser's return.
    OauthWait,
    Restore,
    Enter,
    SignOut,
    Forget,
    Friends,
    Invites,
    SaveProfile,
    OpenProfile(String),
    FindUsers(String),
    OpenPlaylist {
        handle: String,
        id: Uuid,
    },
    FriendAction {
        handle: String,
        add: bool,
    },
    InviteToJam {
        code: String,
        handles: Vec<String>,
    },
    DismissInvite(String),
    /// The track being listened to (`None`: removed from the server).
    Presence {
        track: Option<Uuid>,
        position_ms: u64,
        paused: bool,
    },
    Listen(Uuid),
    PublishPlaylist {
        id: Uuid,
        name: String,
        tracks: Vec<Uuid>,
    },
    UnpublishPlaylist(Uuid),
}

impl SocialWork {
    pub(crate) fn of(job: &social::Job) -> Self {
        use social::Job as J;
        match job {
            J::SignIn { .. } => Self::SignIn,
            J::SignUp { .. } => Self::SignUp,
            J::SubmitCode { .. } => Self::SubmitCode,
            J::ResendCode { .. } => Self::ResendCode,
            J::OauthBegin { provider, .. } => Self::OauthBegin(*provider),
            J::OauthWait { .. } => Self::OauthWait,
            J::Restore { .. } => Self::Restore,
            J::Enter { .. } => Self::Enter,
            J::SignOut { .. } => Self::SignOut,
            J::Forget { .. } => Self::Forget,
            J::Friends { .. } => Self::Friends,
            J::Invites { .. } => Self::Invites,
            J::PutProfile { .. } => Self::SaveProfile,
            J::UserPage { handle, .. } => Self::OpenProfile(handle.clone()),
            J::Users { query, .. } => Self::FindUsers(query.clone()),
            J::UserPlaylist { handle, id, .. } => Self::OpenPlaylist { handle: handle.clone(), id: *id },
            J::FriendAction { handle, add, .. } => Self::FriendAction { handle: handle.clone(), add: *add },
            J::InviteToJam { code, handles, .. } => {
                Self::InviteToJam { code: code.clone(), handles: handles.clone() }
            }
            J::DismissInvite { code, .. } => Self::DismissInvite(code.clone()),
            J::Presence { what, .. } => match what {
                social::Presence::Put { track, position_ms, paused } => {
                    Self::Presence { track: Some(*track), position_ms: *position_ms, paused: *paused }
                }
                social::Presence::Delete => Self::Presence { track: None, position_ms: 0, paused: false },
            },
            J::Listen { track, .. } => Self::Listen(*track),
            J::PutPlaylist { id, name, tracks, .. } => {
                Self::PublishPlaylist { id: *id, name: name.clone(), tracks: tracks.clone() }
            }
            J::DeletePlaylist { id, .. } => Self::UnpublishPlaylist(*id),
        }
    }
}

/// What the core has asked to show to the system or to Discord: in the real core the host runs it.
#[derive(Debug)]
pub enum SystemEffect {
    Media(MediaOp),
    Discord(DiscordOp),
}

/// The test core: state, the last published snapshot and the detached audio output.
pub struct LocalCore {
    state: CoreState,
    snapshot: Arc<Snapshot>,
    audio: Option<AudioOutput>,
    jobs: Vec<effect::Job>,
    system: Vec<SystemEffect>,
    jam: Vec<JamOp>,
    ticking: bool,
    searching: bool,
    downloading: bool,
    resuming: bool,
    polling: bool,
    presence_waiting: bool,
    /// The timer that asks for the full version of the music video is armed.
    #[cfg(feature = "video")]
    video_upgrading: bool,
    browser: Vec<crate::auth::Secret>,
}

impl LocalCore {
    /// An in-memory core with `config` (no file) and the mixes' seed `seed`, without an audio output.
    pub fn new(config: Config, seed: u64) -> Self {
        let state = CoreState::new(config, None, seed);
        let snapshot = Arc::new(state.snapshot());
        Self {
            state,
            snapshot,
            audio: None,
            jobs: Vec::new(),
            system: Vec::new(),
            jam: Vec::new(),
            ticking: false,
            searching: false,
            downloading: false,
            resuming: false,
            polling: false,
            presence_waiting: false,
            #[cfg(feature = "video")]
            video_upgrading: false,
            browser: Vec::new(),
        }
    }

    /// The same core, which says it is release `build` (for the update checks).
    #[must_use]
    pub fn with_build(mut self, build: Option<&'static str>) -> Self {
        self.state.downloads = std::mem::take(&mut self.state.downloads).with_build(build);
        self
    }

    /// The same core with an audio output: the tracks play there.
    #[must_use]
    pub fn with_audio(mut self, audio: AudioOutput) -> Self {
        self.audio = Some(audio);
        self
    }

    /// The audio output, if there is one: the test asks for its state (for example whether the track has ended).
    pub fn audio(&self) -> Option<&AudioOutput> {
        self.audio.as_ref()
    }

    /// The last published snapshot.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshot)
    }

    /// Applies the command; returns the new snapshot if something has changed.
    pub fn send(&mut self, command: Command) -> Option<Arc<Snapshot>> {
        self.run(Input::Command(command));
        self.publish()
    }

    /// The core's tick: it fires only if it is armed (a track plays), as in the real core.
    pub fn tick(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.ticking) {
            self.run(Input::Timer(TimerKey::Playback(playback::Timer::Tick)));
        }
        self.publish()
    }

    /// The tick is armed: the core would wake up shortly.
    pub fn ticking(&self) -> bool {
        self.ticking
    }

    /// The search wait is armed: after the last key the core would launch the search.
    pub fn search_pending(&self) -> bool {
        self.searching
    }

    /// The search wait expires (300 ms after the last key), if it is armed.
    pub fn fire_search(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.searching) {
            self.run(Input::Timer(TimerKey::Catalog(catalog::Timer::Search)));
        }
        self.publish()
    }

    /// The download progress timer is armed.
    pub fn download_progress_pending(&self) -> bool {
        self.downloading
    }

    /// The progress timer expires (every 250 ms while downloading), if it is armed.
    pub fn fire_download_progress(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.downloading) {
            self.run(Input::Timer(TimerKey::Downloads(downloads::Timer::Progress)));
        }
        self.publish()
    }

    /// The pause after a "too many requests" is armed: in twenty seconds the import would resume.
    pub fn import_resume_pending(&self) -> bool {
        self.resuming
    }

    /// The import pause expires (twenty seconds after a "too many requests"), if it is armed.
    pub fn fire_import_resume(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.resuming) {
            self.run(Input::Timer(TimerKey::External(external::Timer::Resume)));
        }
        self.publish()
    }

    /// The friends poll is armed: only with a sign-in and a profile.
    pub fn social_poll_pending(&self) -> bool {
        self.polling
    }

    /// The friends poll expires (every 15 seconds with the sign-in done), if it is armed.
    pub fn fire_social_poll(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.polling) {
            self.run(Input::Timer(TimerKey::Social(social::Timer::Poll)));
        }
        self.publish()
    }

    /// The deferral of a position jump, after a send too close, is armed.
    pub fn presence_wait_pending(&self) -> bool {
        self.presence_waiting
    }

    /// The jump wait expires, if it is armed.
    pub fn fire_presence_wait(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.presence_waiting) {
            self.run(Input::Timer(TimerKey::Social(social::Timer::Presence)));
        }
        self.publish()
    }

    /// The timer that asks whether the full version of the music video is ready is armed: only with a
    /// music video shown and still below 720p.
    #[cfg(feature = "video")]
    pub fn video_upgrade_pending(&self) -> bool {
        self.video_upgrading
    }

    /// The wait for the music video's full version expires (6 seconds after the last reply), if it is
    /// armed.
    #[cfg(feature = "video")]
    pub fn fire_video_upgrade(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.video_upgrading) {
            self.run(Input::Timer(TimerKey::Video(crate::state::video::Timer::Upgrade)));
        }
        self.publish()
    }

    /// The server has answered the request `ticket` for the music video's address
    /// (`PendingJob::VideoUrl`).
    #[cfg(feature = "video")]
    pub fn video_url_loaded(
        &mut self,
        ticket: u64,
        result: Result<crate::api::SignedUrl, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.run(Input::Done(Done::Video(crate::state::video::Done::Url { ticket, result })));
        self.publish()
    }

    /// The addresses the core has asked to open in the browser since last time. Here no browser
    /// opens: in the real core the host opens them with `platform::open_url`.
    pub fn take_browser(&mut self) -> Vec<crate::auth::Secret> {
        std::mem::take(&mut self.browser)
    }

    /// The shared progress of the download in progress for `track`: the test writes to it as the
    /// work would while the bytes arrive.
    pub fn download_progress(&self, track: Uuid) -> Option<Arc<Progress>> {
        self.state.downloads.progress_of(track)
    }

    /// The download of `track` has finished (or not).
    pub fn downloaded(&mut self, track: Uuid, result: Result<Downloaded, AppError>) -> Option<Arc<Snapshot>> {
        let ticket = self.state.downloads.ticket(track)?;
        self.downloads_done(downloads::Done::Track { id: track, ticket, result })
    }

    /// The update check has come back.
    pub fn update_checked(
        &mut self,
        kind: UpdateCheck,
        result: Result<Option<Offer>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::UpdateChecked { kind, result })
    }

    /// The check of the downloaded files has come back: the tracks checked and those not in order.
    pub fn verified(&mut self, deep: bool, checked: usize, lost: Vec<Lost>) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::Verified { deep, checked, lost })
    }

    /// The file of a removed track has been deleted (`Ok(false)`), left because it changed (`Ok(true)`),
    /// or could not be deleted.
    pub fn removed(&mut self, title: &str, result: Result<bool, AppError>) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::Removed { title: title.to_owned(), result })
    }

    /// The downloads folder has opened (or not).
    pub fn folder_opened(&mut self, opened: bool) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::FolderOpened(opened))
    }

    /// The update package's folder has opened (or not).
    pub fn update_revealed(&mut self, opened: bool) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::UpdateRevealed(opened))
    }

    /// The update package has arrived (or not).
    pub fn update_downloaded(&mut self, result: Result<PathBuf, AppError>) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::UpdateDownloaded(result))
    }

    /// The network jobs asked since last time, and removes them from the queue.
    pub fn take_jobs(&mut self) -> Vec<PendingJob> {
        let jobs = std::mem::take(&mut self.jobs);
        jobs.iter()
            .filter_map(|job| match job {
                effect::Job::Playback(playback::Job::Load { track_id, generation, .. }) => {
                    Some(PendingJob::Load { track: *track_id, generation: *generation })
                }
                effect::Job::Playback(playback::Job::Warm { track_id, whole, .. }) => {
                    Some(PendingJob::Warm { track: *track_id, whole: *whole })
                }
                effect::Job::Playback(playback::Job::Autoplay { artist, .. }) => {
                    Some(PendingJob::Autoplay { artist: *artist })
                }
                effect::Job::Playback(playback::Job::Lyrics { track_id, .. }) => {
                    Some(PendingJob::Lyrics { track: *track_id })
                }
                effect::Job::Playback(playback::Job::Album { id, ticket, .. }) => {
                    Some(PendingJob::Album { id: *id, ticket: *ticket })
                }
                effect::Job::Playback(playback::Job::Radio { start, ticket, .. }) => {
                    Some(PendingJob::Radio { start: start.id, artist: start.artist.id, ticket: *ticket })
                }
                effect::Job::Social(job) => Some(PendingJob::Social(SocialWork::of(job))),
                #[cfg(feature = "video")]
                effect::Job::Video(job) => Some(match job {
                    crate::state::video::Job::Url { track, ticket, .. } => {
                        PendingJob::VideoUrl { track: *track, ticket: *ticket }
                    }
                    crate::state::video::Job::Prepare { id, .. } => PendingJob::VideoPrepare { track: *id },
                }),
                effect::Job::Jam(jam::Job::Tracks { ids, .. }) => {
                    Some(PendingJob::JamTracks { ids: ids.clone() })
                }
                effect::Job::Downloads(job) => Some(match job {
                    downloads::Job::Track { id, .. } => PendingJob::Download { track: *id },
                    downloads::Job::Discard(_) => PendingJob::Discard,
                    downloads::Job::CheckUpdate { kind, .. } => PendingJob::CheckUpdate { kind: *kind },
                    downloads::Job::DownloadUpdate { .. } => PendingJob::DownloadUpdate,
                    downloads::Job::Verify { deep, .. } => PendingJob::Verify { deep: *deep },
                    downloads::Job::Remove { title, .. } => PendingJob::Remove { title: title.clone() },
                    downloads::Job::OpenFolder(_) => PendingJob::OpenFolder,
                    downloads::Job::RevealUpdate(_) => PendingJob::RevealUpdate,
                }),
                effect::Job::External(job) => Some(match job {
                    external::Job::Playlist { ticket, source, id, .. } => {
                        PendingJob::ExternalPlaylist { source: *source, id: id.clone(), ticket: *ticket }
                    }
                    external::Job::Import { target, key, video, .. } => {
                        PendingJob::Import { target: *target, key: key.clone(), video: *video }
                    }
                    external::Job::Artist { id, name, .. } => {
                        PendingJob::ArtistExtras { id: *id, name: name.clone() }
                    }
                }),
                effect::Job::Catalog(job) => Some(match job {
                    catalog::Job::Home { section, .. } => PendingJob::Home(*section),
                    catalog::Job::HomeAlbums { .. } => PendingJob::HomeAlbums,
                    catalog::Job::Search { ticket, query, .. } => {
                        PendingJob::Search { ticket: *ticket, query: query.clone() }
                    }
                    catalog::Job::Discover { ticket, query, .. } => {
                        PendingJob::Discover { ticket: *ticket, query: query.clone() }
                    }
                    catalog::Job::Meta { ticket, query, .. } => {
                        PendingJob::Meta { ticket: *ticket, query: query.clone() }
                    }
                    catalog::Job::LibraryTracks { offset, append, .. } => {
                        PendingJob::LibraryTracks { offset: *offset, append: *append }
                    }
                    catalog::Job::LibraryAlbums { .. } => PendingJob::LibraryAlbums,
                    catalog::Job::Album { id, silent, .. } => {
                        PendingJob::CatalogAlbum { id: *id, silent: *silent }
                    }
                    catalog::Job::Artist { id, fetch, .. } => {
                        PendingJob::CatalogArtist { id: *id, background: *fetch == Fetch::Background }
                    }
                    catalog::Job::ArchiveLibrary { identifier, page, .. } => {
                        PendingJob::ArchiveLibrary { identifier: identifier.clone(), page: *page }
                    }
                    catalog::Job::MetaArtist { id, .. } => PendingJob::MetaArtist { id: *id },
                    catalog::Job::Heal { track, .. } => PendingJob::Heal { track: track.id },
                    catalog::Job::Refresh { id, .. } => PendingJob::Refresh { id: *id },
                }),
                _ => None,
            })
            .collect()
    }

    /// The server has answered the load `generation`.
    pub fn loaded(
        &mut self,
        generation: u64,
        result: Result<StreamBuffer, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Loaded { generation, result, lost: None })
    }

    /// A track prepared in advance has arrived (or did not open).
    pub fn warmed(
        &mut self,
        track: Uuid,
        quality: Quality,
        result: Result<StreamBuffer, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Warmed { track_id: track, quality, result })
    }

    /// The server has answered the request for the tracks of automatic playback.
    pub fn autoplay_loaded(&mut self, result: Result<ArtistPage, AppError>) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Autoplay(result.map(Box::new)))
    }

    /// The text of `track` has arrived (or not).
    pub fn lyrics_loaded(
        &mut self,
        track: Uuid,
        result: Result<Arc<Text>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Lyrics { track_id: track, result })
    }

    /// The server has answered the request for album number `ticket`.
    pub fn album_loaded(&mut self, ticket: u64, result: Result<Album, AppError>) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Album { ticket, result: result.map(Box::new) })
    }

    /// The server has answered the request for the radio's artist number `ticket`.
    pub fn radio_loaded(
        &mut self,
        ticket: u64,
        start: Track,
        result: Result<ArtistPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Radio { ticket, start: Box::new(start), result: result.map(Box::new) })
    }

    /// The server answers a Home shelf (the replies belong to the request in progress).
    pub fn home_loaded(
        &mut self,
        section: Section,
        result: Result<Page<Track>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        let epoch = self.state.catalog.home_epoch();
        self.catalog_done(catalog::Done::Home { epoch, section, result })
    }

    pub fn home_albums_loaded(
        &mut self,
        result: Result<Page<AlbumSummary>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        let epoch = self.state.catalog.home_epoch();
        self.catalog_done(catalog::Done::HomeAlbums { epoch, result })
    }

    /// The search number `ticket` has answered from the catalog.
    pub fn searched(
        &mut self,
        ticket: u64,
        result: Result<SearchResults, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Search { ticket, result: result.map(Box::new) })
    }

    /// The search number `ticket` has answered from Internet Archive.
    pub fn discovered(
        &mut self,
        ticket: u64,
        result: Result<DiscoverPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Discover { ticket, result: result.map(Box::new) })
    }

    /// The search number `ticket` has answered from the official catalog.
    pub fn meta_searched(
        &mut self,
        ticket: u64,
        result: Result<MetaSearch, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Meta { ticket, result: result.map(Box::new) })
    }

    pub fn library_tracks_loaded(
        &mut self,
        append: bool,
        result: Result<Page<Track>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        let epoch = self.state.catalog.library_epoch();
        self.catalog_done(catalog::Done::LibraryTracks { epoch, append, result })
    }

    pub fn library_albums_loaded(
        &mut self,
        result: Result<Page<AlbumSummary>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        let epoch = self.state.catalog.library_epoch();
        self.catalog_done(catalog::Done::LibraryAlbums { epoch, result })
    }

    /// An album to show has arrived.
    pub fn catalog_album(&mut self, id: Uuid, result: Result<Album, AppError>) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Album { id, silent: false, result: result.map(Box::new) })
    }

    /// An artist page has arrived, from an opening (`background` false) or early.
    pub fn catalog_artist(
        &mut self,
        id: Uuid,
        background: bool,
        result: Result<ArtistPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        let fetch = if background { Fetch::Background } else { Fetch::Open };
        self.catalog_done(catalog::Done::Artist { id, fetch, result: result.map(Box::new) })
    }

    /// An album page of an Archive library has arrived.
    pub fn archive_page(
        &mut self,
        identifier: &str,
        page: u32,
        result: Result<LibraryPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::ArchiveLibrary { identifier: identifier.to_owned(), page, result })
    }

    /// The official catalog's artist now has an identifier.
    pub fn meta_artist_opened(&mut self, result: Result<Uuid, AppError>) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::MetaArtist(result))
    }

    /// The track `old` has been searched for in the official catalog.
    pub fn healed(&mut self, old: Uuid, result: Result<Track, AppError>) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Healed { old, result: result.map(Box::new) })
    }

    /// A track reread from the server.
    pub fn refreshed(&mut self, result: Result<Track, AppError>) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Refreshed(result.map(Box::new)))
    }

    /// The tracks of an external playlist have arrived (or not), for the opening number `ticket`.
    pub fn playlist_loaded(
        &mut self,
        ticket: u64,
        source: SourceKind,
        result: Result<SourcePlaylistPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.external_done(external::Done::Playlist { ticket, source, result: result.map(Box::new) })
    }

    /// An import has finished: `target` is that of the job asked for (`PendingJob::Import`).
    pub fn imported(
        &mut self,
        target: ImportTarget,
        result: Result<DiscoverImport, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.external_done(external::Done::Imported { target, result: result.map(Box::new) })
    }

    /// The artist's "Altro di …" sheet has arrived (or not).
    pub fn artist_extras_loaded(
        &mut self,
        id: Uuid,
        result: Result<MetaArtistPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.external_done(external::Done::Artist { id, result: result.map(Box::new) })
    }

    /// The friends and the requests have arrived (or not).
    pub fn friends_loaded(&mut self, result: Result<Friends, AppError>) -> Option<Arc<Snapshot>> {
        let ticket = self.state.social.friends_ticket();
        self.social_done(social::Done::Friends { ticket, result })
    }

    /// Clerk has asked for a code for the sign-in attempt in progress (`sent_to`: where it arrived).
    pub fn sign_in_needs_code(&mut self, sent_to: &str) -> Option<Arc<Snapshot>> {
        let attempt = self.state.social.attempt();
        let step = crate::auth::Step::Code(crate::auth::CodeStep::for_tests(sent_to));
        self.social_done(social::Done::Stepped { attempt, result: Ok(Box::new(step)) })
    }

    /// The sign-in attempt in progress has failed.
    pub fn sign_in_failed(&mut self, error: AppError) -> Option<Arc<Snapshot>> {
        let attempt = self.state.social.attempt();
        self.social_done(social::Done::Stepped { attempt, result: Err(error) })
    }

    /// The sign-in is complete (not a restore): the core says hello and reads the friends.
    pub fn signed_in(&mut self, session: crate::auth::Session, me: crate::api::Me) -> Option<Arc<Snapshot>> {
        let result = Ok(Box::new((session, crate::auth::ClerkUser::default(), me)));
        self.social_done(social::Done::Entered { restored: false, result })
    }

    /// The jam invitations have arrived (or not).
    pub fn invites_loaded(&mut self, result: Result<Vec<JamInvite>, AppError>) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::Invites(result))
    }

    /// The profile has been saved (or not).
    pub fn profile_saved(&mut self, result: Result<MyProfile, AppError>) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::ProfileSaved(result))
    }

    /// A send that shows nothing (track being listened to, play, playlist) has finished.
    pub fn social_synced(&mut self, result: Result<(), AppError>) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::Sync(result))
    }

    /// A user's profile has arrived (or not).
    pub fn user_opened(&mut self, handle: &str, result: Result<UserPage, AppError>) -> Option<Arc<Snapshot>> {
        let result = result.map(|page| {
            let recent: Vec<Track> = page
                .activity
                .as_ref()
                .map(|a| a.recent.iter().map(|l| l.track.clone()).collect())
                .unwrap_or_default();
            Box::new(social::OpenProfile { page, recent: Arc::from(recent) })
        });
        self.social_done(social::Done::ProfileLoaded { handle: handle.to_owned(), result })
    }

    /// The people search has answered.
    pub fn users_found(&mut self, result: Result<Vec<Profile>, AppError>) -> Option<Arc<Snapshot>> {
        let ticket = self.state.social.found_ticket();
        self.social_done(social::Done::Found { ticket, result })
    }

    /// A friendship action has finished.
    pub fn friend_changed(
        &mut self,
        handle: &str,
        result: Result<Relation, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::FriendChanged { handle: handle.to_owned(), result })
    }

    /// A friend's public playlist has arrived (or not).
    pub fn friend_playlist_loaded(
        &mut self,
        result: Result<Arc<PublicPlaylist>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        let ticket = self.state.social.playlist_ticket();
        self.social_done(social::Done::PlaylistLoaded { ticket, result })
    }

    /// The jam invitations have been sent (or not).
    pub fn invited(&mut self, result: Result<usize, AppError>) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::Invited(result))
    }

    /// A command from the system (a media key), as the host would have given it.
    pub fn media(&mut self, event: media::Event) -> Option<Arc<Snapshot>> {
        self.run(Input::Media(event));
        self.publish()
    }

    /// An event from the jam room, as the link to the room would have given it to the host.
    pub fn jam_event(&mut self, event: crate::jam::Event) -> Option<Arc<Snapshot>> {
        self.run(Input::Jam(event));
        self.publish()
    }

    /// The catalog tracks asked for the shared jam queue have arrived (or not).
    pub fn jam_tracks_loaded(&mut self, result: Result<Vec<Track>, AppError>) -> Option<Arc<Snapshot>> {
        let ticket = self.state.jam.fetch_ticket()?;
        self.run(Input::Done(Done::Jam(jam::Done::Tracks { ticket, result })));
        self.publish()
    }

    /// What the jam has asked of the link to the room since last time (start it, send the local state,
    /// close it): in the real core the host runs it.
    pub fn take_jam(&mut self) -> Vec<JamOp> {
        std::mem::take(&mut self.jam)
    }

    /// What the core has asked to show to the system or to Discord since last time.
    pub fn take_system(&mut self) -> Vec<SystemEffect> {
        std::mem::take(&mut self.system)
    }

    fn social_done(&mut self, done: social::Done) -> Option<Arc<Snapshot>> {
        self.run(Input::Done(Done::Social(done)));
        self.publish()
    }

    fn external_done(&mut self, done: external::Done) -> Option<Arc<Snapshot>> {
        self.run(Input::Done(Done::External(done)));
        self.publish()
    }

    fn downloads_done(&mut self, done: downloads::Done) -> Option<Arc<Snapshot>> {
        self.run(Input::Done(Done::Downloads(done)));
        self.publish()
    }

    fn catalog_done(&mut self, done: catalog::Done) -> Option<Arc<Snapshot>> {
        self.run(Input::Done(Done::Catalog(done)));
        self.publish()
    }

    fn done(&mut self, done: playback::Done) -> Option<Arc<Snapshot>> {
        self.run(Input::Done(Done::Playback(done)));
        self.publish()
    }

    /// Gives the input to the reducer and runs what it asks, the audio included: what the audio
    /// answers goes back in at once, as in the host.
    fn run(&mut self, input: Input) {
        let now = Instant::now();
        let mut pending = VecDeque::from([input]);
        while let Some(input) = pending.pop_front() {
            for effect in self.state.apply(input, now) {
                match effect {
                    Effect::Audio(op) => pending.extend(run_audio(&mut self.audio, op).map(Input::Audio)),
                    Effect::Run(job) => self.jobs.push(job),
                    Effect::Jam(op) => self.jam.push(op),
                    Effect::Media(op) => self.system.push(SystemEffect::Media(op)),
                    Effect::Discord(op) => self.system.push(SystemEffect::Discord(op)),
                    Effect::OpenUrl(url) => self.browser.push(url),
                    Effect::Timer { key: TimerKey::Social(social::Timer::Poll), .. } => self.polling = true,
                    Effect::CancelTimer(TimerKey::Social(social::Timer::Poll)) => self.polling = false,
                    Effect::Timer { key: TimerKey::Social(social::Timer::Presence), .. } => {
                        self.presence_waiting = true;
                    }
                    Effect::CancelTimer(TimerKey::Social(social::Timer::Presence)) => {
                        self.presence_waiting = false;
                    }
                    #[cfg(feature = "video")]
                    Effect::Timer { key: TimerKey::Video(_), .. } => self.video_upgrading = true,
                    #[cfg(feature = "video")]
                    Effect::CancelTimer(TimerKey::Video(_)) => self.video_upgrading = false,
                    Effect::Timer { key: TimerKey::Playback(_), .. } => self.ticking = true,
                    Effect::CancelTimer(TimerKey::Playback(_)) => self.ticking = false,
                    Effect::Timer { key: TimerKey::External(_), .. } => self.resuming = true,
                    Effect::CancelTimer(TimerKey::External(_)) => self.resuming = false,
                    Effect::Timer { key: TimerKey::Catalog(_), .. } => self.searching = true,
                    Effect::CancelTimer(TimerKey::Catalog(_)) => self.searching = false,
                    Effect::Timer { key: TimerKey::Downloads(downloads::Timer::Progress), .. } => {
                        self.downloading = true;
                    }
                    Effect::CancelTimer(TimerKey::Downloads(downloads::Timer::Progress)) => {
                        self.downloading = false;
                    }
                    // Nothing writes to disk, and no other timer is needed for the frontend tests.
                    _ => {}
                }
            }
        }
    }

    fn publish(&mut self) -> Option<Arc<Snapshot>> {
        let next = Arc::new(self.state.publish(&self.snapshot)?);
        self.snapshot = Arc::clone(&next);
        Some(next)
    }
}
