//! Un core sincrono per i test dei frontend: lo stesso riduttore del core vero, ma nel thread del
//! test. Un comando si applica subito e restituisce l'istantanea che il core avrebbe pubblicato,
//! senza thread, canali né runtime. Gli effetti si eseguono qui, quelli che non toccano il mondo:
//! l'audio (un'uscita staccata, da cui il test legge i campioni) e il battito, che il test fa
//! scoccare con [`LocalCore::tick`]; i lavori di rete si raccolgono per dire che cosa è stato
//! chiesto ([`LocalCore::take_jobs`]) e il test ne consegna l'esito a mano. Niente scrive sul disco.
//!
//! Serve ai test che guidano un frontend con una sequenza di messaggi e guardano il suo stato (il
//! `config.json` e il resto del core vero si provano con `Core::start`). Un build vero non lo
//! contiene: lo accende solo la feature `test-support`.

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

/// Un lavoro di rete che il core ha chiesto e che il test non ha ancora risolto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingJob {
    /// Aprire il brano `track` per il caricamento `generation`.
    Load {
        track: Uuid,
        generation: u64,
    },
    /// Preparare un brano: la testata, o per intero (`whole`).
    Warm {
        track: Uuid,
        whole: bool,
    },
    /// Altri brani dell'artista, per la riproduzione automatica.
    Autoplay {
        artist: Uuid,
    },
    /// Il testo del brano `track`.
    Lyrics {
        track: Uuid,
    },
    /// I brani dell'album `id`; `ticket` è il numero della richiesta, da restituire con la risposta.
    Album {
        id: Uuid,
        ticket: u64,
    },
    /// L'artista di una radio partita da `start`.
    Radio {
        start: Uuid,
        artist: Uuid,
        ticket: u64,
    },
    /// Uno scaffale della Home (recenti o alfabetico).
    Home(Section),
    /// Gli album della Home.
    HomeAlbums,
    /// Le tre ricerche del testo `query`, ciascuna con il numero della richiesta.
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
    /// Una pagina di brani della libreria, dal brano `offset`.
    LibraryTracks {
        offset: u32,
        append: bool,
    },
    LibraryAlbums,
    /// Un album da mostrare; `silent` se è un aggiornamento in silenzio.
    CatalogAlbum {
        id: Uuid,
        silent: bool,
    },
    /// Una pagina di artista; `background` se è in anticipo o un aggiornamento in silenzio.
    CatalogArtist {
        id: Uuid,
        background: bool,
    },
    /// Una pagina degli album di una libreria di Archive.
    ArchiveLibrary {
        identifier: String,
        page: u32,
    },
    /// Un lavoro dell'account, del profilo, degli amici o della presenza.
    Social(SocialWork),
    /// L'artista del catalogo ufficiale da portare nel server.
    MetaArtist {
        id: u64,
    },
    /// Un brano sparito da ritrovare nel catalogo.
    Heal {
        track: Uuid,
    },
    /// Un brano da rileggere dal server.
    Refresh {
        id: Uuid,
    },
    /// Un brano da scaricare.
    Download {
        track: Uuid,
    },
    /// Un file di un download annullato da togliere dal disco.
    Discard,
    /// L'ultima release da leggere.
    CheckUpdate {
        kind: UpdateCheck,
    },
    /// Il pacchetto dell'aggiornamento da scaricare.
    DownloadUpdate,
    /// I file scaricati da controllare (`deep`: anche il contenuto).
    Verify {
        deep: bool,
    },
    /// Il file del brano `title`, tolto dall'indice, da togliere dal disco.
    Remove {
        title: String,
    },
    /// La cartella dei download da aprire.
    OpenFolder,
    /// La cartella del pacchetto dell'aggiornamento da aprire.
    RevealUpdate,
    /// I brani di una playlist esterna; `ticket` è il numero dell'apertura, da restituire.
    ExternalPlaylist {
        source: SourceKind,
        id: String,
        ticket: u64,
    },
    /// Un brano (o un item di Archive) da portare nel catalogo, e dove andrà.
    Import {
        target: ImportTarget,
        key: Importing,
        video: bool,
    },
    /// I brani popolari e la discografia di un artista dal catalogo ufficiale.
    ArtistExtras {
        id: Uuid,
        name: String,
    },
    /// I brani del catalogo che la coda condivisa della jam ha e quella locale no.
    JamTracks {
        ids: Vec<Uuid>,
    },
    /// L'indirizzo del videoclip di `track`, per la richiesta `ticket`.
    #[cfg(feature = "video")]
    VideoUrl {
        track: Uuid,
        ticket: u64,
    },
    /// Il server deve preparare il videoclip di `track`.
    #[cfg(feature = "video")]
    VideoPrepare {
        track: Uuid,
    },
}

/// Un lavoro dell'account o del profilo che il core ha chiesto: quanto basta a un test per dire che
/// cosa è partito (mai i segreti: password, codici e token non ci sono).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocialWork {
    SignIn,
    SignUp,
    SubmitCode,
    ResendCode,
    /// L'accesso con il browser: il core mette in ascolto il loopback e chiede a Clerk il provider.
    OauthBegin(crate::auth::Provider),
    /// L'attesa del ritorno del browser.
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
    /// Il brano in ascolto (`None`: tolto dal server).
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

/// Ciò che il core ha chiesto di mostrare al sistema o a Discord: nel core vero lo esegue l'host.
#[derive(Debug)]
pub enum SystemEffect {
    Media(MediaOp),
    Discord(DiscordOp),
}

/// Il core dei test: stato, l'ultima istantanea pubblicata e l'uscita audio staccata.
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
    /// Il timer che chiede la versione completa del videoclip è armato.
    #[cfg(feature = "video")]
    video_upgrading: bool,
    browser: Vec<crate::auth::Secret>,
}

impl LocalCore {
    /// Un core in memoria con `config` (niente file) e il seme dei mix `seed`, senza uscita audio.
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

    /// Lo stesso core, che dice di essere la release `build` (per i controlli degli aggiornamenti).
    #[must_use]
    pub fn with_build(mut self, build: Option<&'static str>) -> Self {
        self.state.downloads = std::mem::take(&mut self.state.downloads).with_build(build);
        self
    }

    /// Lo stesso core con un'uscita audio: i brani suonano lì.
    #[must_use]
    pub fn with_audio(mut self, audio: AudioOutput) -> Self {
        self.audio = Some(audio);
        self
    }

    /// L'uscita audio, se c'è: il test ne chiede lo stato (per esempio se il brano è finito).
    pub fn audio(&self) -> Option<&AudioOutput> {
        self.audio.as_ref()
    }

    /// L'ultima istantanea pubblicata.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshot)
    }

    /// Applica il comando; restituisce la nuova istantanea se qualcosa è cambiato.
    pub fn send(&mut self, command: Command) -> Option<Arc<Snapshot>> {
        self.run(Input::Command(command));
        self.publish()
    }

    /// Il battito del core: scocca solo se è armato (suona un brano), come nel core vero.
    pub fn tick(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.ticking) {
            self.run(Input::Timer(TimerKey::Playback(playback::Timer::Tick)));
        }
        self.publish()
    }

    /// Il battito è armato: il core si risveglierebbe fra poco.
    pub fn ticking(&self) -> bool {
        self.ticking
    }

    /// L'attesa della ricerca è armata: dopo l'ultimo tasto il core lancerebbe la ricerca.
    pub fn search_pending(&self) -> bool {
        self.searching
    }

    /// Scade l'attesa della ricerca (300 ms dopo l'ultimo tasto), se è armata.
    pub fn fire_search(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.searching) {
            self.run(Input::Timer(TimerKey::Catalog(catalog::Timer::Search)));
        }
        self.publish()
    }

    /// Il timer dell'avanzamento dei download è armato.
    pub fn download_progress_pending(&self) -> bool {
        self.downloading
    }

    /// Scade il timer dell'avanzamento (ogni 250 ms mentre si scarica), se è armato.
    pub fn fire_download_progress(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.downloading) {
            self.run(Input::Timer(TimerKey::Downloads(downloads::Timer::Progress)));
        }
        self.publish()
    }

    /// La pausa dopo un «troppe richieste» è armata: fra venti secondi l'importazione riprenderebbe.
    pub fn import_resume_pending(&self) -> bool {
        self.resuming
    }

    /// Scade la pausa dell'importazione (venti secondi dopo un «troppe richieste»), se è armata.
    pub fn fire_import_resume(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.resuming) {
            self.run(Input::Timer(TimerKey::External(external::Timer::Resume)));
        }
        self.publish()
    }

    /// Il sondaggio degli amici è armato: solo con un accesso e un profilo.
    pub fn social_poll_pending(&self) -> bool {
        self.polling
    }

    /// Scade il sondaggio degli amici (ogni 15 secondi con l'accesso fatto), se è armato.
    pub fn fire_social_poll(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.polling) {
            self.run(Input::Timer(TimerKey::Social(social::Timer::Poll)));
        }
        self.publish()
    }

    /// Il rinvio di un salto di posizione, dopo un invio troppo vicino, è armato.
    pub fn presence_wait_pending(&self) -> bool {
        self.presence_waiting
    }

    /// Scade l'attesa del salto, se è armata.
    pub fn fire_presence_wait(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.presence_waiting) {
            self.run(Input::Timer(TimerKey::Social(social::Timer::Presence)));
        }
        self.publish()
    }

    /// Il timer che chiede se la versione completa del videoclip è pronta è armato: solo con un
    /// videoclip mostrato e ancora sotto i 720p.
    #[cfg(feature = "video")]
    pub fn video_upgrade_pending(&self) -> bool {
        self.video_upgrading
    }

    /// Scade l'attesa della versione completa del videoclip (6 secondi dopo l'ultima risposta), se è
    /// armata.
    #[cfg(feature = "video")]
    pub fn fire_video_upgrade(&mut self) -> Option<Arc<Snapshot>> {
        if std::mem::take(&mut self.video_upgrading) {
            self.run(Input::Timer(TimerKey::Video(crate::state::video::Timer::Upgrade)));
        }
        self.publish()
    }

    /// Il server ha risposto alla richiesta `ticket` dell'indirizzo del videoclip
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

    /// Gli indirizzi che il core ha chiesto di aprire nel browser dall'ultima volta. Qui nessun
    /// browser si apre: nel core vero li apre l'host con `platform::open_url`.
    pub fn take_browser(&mut self) -> Vec<crate::auth::Secret> {
        std::mem::take(&mut self.browser)
    }

    /// L'avanzamento condiviso del download in corso di `track`: il test ci scrive come farebbe il
    /// lavoro mentre i byte arrivano.
    pub fn download_progress(&self, track: Uuid) -> Option<Arc<Progress>> {
        self.state.downloads.progress_of(track)
    }

    /// Il download di `track` è finito (o no).
    pub fn downloaded(&mut self, track: Uuid, result: Result<Downloaded, AppError>) -> Option<Arc<Snapshot>> {
        let ticket = self.state.downloads.ticket(track)?;
        self.downloads_done(downloads::Done::Track { id: track, ticket, result })
    }

    /// Il controllo degli aggiornamenti è tornato.
    pub fn update_checked(
        &mut self,
        kind: UpdateCheck,
        result: Result<Option<Offer>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::UpdateChecked { kind, result })
    }

    /// Il controllo dei file scaricati è tornato: i brani controllati e quelli non a posto.
    pub fn verified(&mut self, deep: bool, checked: usize, lost: Vec<Lost>) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::Verified { deep, checked, lost })
    }

    /// Il file di un brano tolto è stato cancellato (`Ok(false)`), lasciato perché cambiato (`Ok(true)`),
    /// o non si è potuto cancellare.
    pub fn removed(&mut self, title: &str, result: Result<bool, AppError>) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::Removed { title: title.to_owned(), result })
    }

    /// La cartella dei download si è aperta (o no).
    pub fn folder_opened(&mut self, opened: bool) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::FolderOpened(opened))
    }

    /// La cartella del pacchetto dell'aggiornamento si è aperta (o no).
    pub fn update_revealed(&mut self, opened: bool) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::UpdateRevealed(opened))
    }

    /// Il pacchetto dell'aggiornamento è arrivato (o no).
    pub fn update_downloaded(&mut self, result: Result<PathBuf, AppError>) -> Option<Arc<Snapshot>> {
        self.downloads_done(downloads::Done::UpdateDownloaded(result))
    }

    /// I lavori di rete chiesti dall'ultima volta, e li toglie dalla coda.
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

    /// Il server ha risposto al caricamento `generation`.
    pub fn loaded(
        &mut self,
        generation: u64,
        result: Result<StreamBuffer, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Loaded { generation, result, lost: None })
    }

    /// Un brano preparato in anticipo è arrivato (o non si è aperto).
    pub fn warmed(
        &mut self,
        track: Uuid,
        quality: Quality,
        result: Result<StreamBuffer, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Warmed { track_id: track, quality, result })
    }

    /// Il server ha risposto alla richiesta dei brani per la riproduzione automatica.
    pub fn autoplay_loaded(&mut self, result: Result<ArtistPage, AppError>) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Autoplay(result.map(Box::new)))
    }

    /// Il testo di `track` è arrivato (o no).
    pub fn lyrics_loaded(
        &mut self,
        track: Uuid,
        result: Result<Arc<Text>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Lyrics { track_id: track, result })
    }

    /// Il server ha risposto alla richiesta dell'album numero `ticket`.
    pub fn album_loaded(&mut self, ticket: u64, result: Result<Album, AppError>) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Album { ticket, result: result.map(Box::new) })
    }

    /// Il server ha risposto alla richiesta dell'artista della radio numero `ticket`.
    pub fn radio_loaded(
        &mut self,
        ticket: u64,
        start: Track,
        result: Result<ArtistPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.done(playback::Done::Radio { ticket, start: Box::new(start), result: result.map(Box::new) })
    }

    /// Il server risponde a uno scaffale della Home (le risposte sono della richiesta in corso).
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

    /// La ricerca numero `ticket` ha risposto nel catalogo.
    pub fn searched(
        &mut self,
        ticket: u64,
        result: Result<SearchResults, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Search { ticket, result: result.map(Box::new) })
    }

    /// La ricerca numero `ticket` ha risposto su Internet Archive.
    pub fn discovered(
        &mut self,
        ticket: u64,
        result: Result<DiscoverPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Discover { ticket, result: result.map(Box::new) })
    }

    /// La ricerca numero `ticket` ha risposto nel catalogo ufficiale.
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

    /// Un album da mostrare è arrivato.
    pub fn catalog_album(&mut self, id: Uuid, result: Result<Album, AppError>) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Album { id, silent: false, result: result.map(Box::new) })
    }

    /// Una pagina di artista è arrivata, da un'apertura (`background` falso) o in anticipo.
    pub fn catalog_artist(
        &mut self,
        id: Uuid,
        background: bool,
        result: Result<ArtistPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        let fetch = if background { Fetch::Background } else { Fetch::Open };
        self.catalog_done(catalog::Done::Artist { id, fetch, result: result.map(Box::new) })
    }

    /// Una pagina di album di una libreria di Archive è arrivata.
    pub fn archive_page(
        &mut self,
        identifier: &str,
        page: u32,
        result: Result<LibraryPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::ArchiveLibrary { identifier: identifier.to_owned(), page, result })
    }

    /// L'artista del catalogo ufficiale ora ha un identificatore.
    pub fn meta_artist_opened(&mut self, result: Result<Uuid, AppError>) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::MetaArtist(result))
    }

    /// Il brano `old` è stato cercato nel catalogo ufficiale.
    pub fn healed(&mut self, old: Uuid, result: Result<Track, AppError>) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Healed { old, result: result.map(Box::new) })
    }

    /// Un brano riletto dal server.
    pub fn refreshed(&mut self, result: Result<Track, AppError>) -> Option<Arc<Snapshot>> {
        self.catalog_done(catalog::Done::Refreshed(result.map(Box::new)))
    }

    /// I brani di una playlist esterna sono arrivati (o no), per l'apertura numero `ticket`.
    pub fn playlist_loaded(
        &mut self,
        ticket: u64,
        source: SourceKind,
        result: Result<SourcePlaylistPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.external_done(external::Done::Playlist { ticket, source, result: result.map(Box::new) })
    }

    /// Un'importazione è finita: `target` è quello del lavoro chiesto (`PendingJob::Import`).
    pub fn imported(
        &mut self,
        target: ImportTarget,
        result: Result<DiscoverImport, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.external_done(external::Done::Imported { target, result: result.map(Box::new) })
    }

    /// La scheda «Altro di …» di un artista è arrivata (o no).
    pub fn artist_extras_loaded(
        &mut self,
        id: Uuid,
        result: Result<MetaArtistPage, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.external_done(external::Done::Artist { id, result: result.map(Box::new) })
    }

    /// Gli amici e le richieste sono arrivati (o no).
    pub fn friends_loaded(&mut self, result: Result<Friends, AppError>) -> Option<Arc<Snapshot>> {
        let ticket = self.state.social.friends_ticket();
        self.social_done(social::Done::Friends { ticket, result })
    }

    /// Clerk ha chiesto un codice per il tentativo di accesso in corso (`sent_to`: dove è arrivato).
    pub fn sign_in_needs_code(&mut self, sent_to: &str) -> Option<Arc<Snapshot>> {
        let attempt = self.state.social.attempt();
        let step = crate::auth::Step::Code(crate::auth::CodeStep::for_tests(sent_to));
        self.social_done(social::Done::Stepped { attempt, result: Ok(Box::new(step)) })
    }

    /// Il tentativo di accesso in corso non è riuscito.
    pub fn sign_in_failed(&mut self, error: AppError) -> Option<Arc<Snapshot>> {
        let attempt = self.state.social.attempt();
        self.social_done(social::Done::Stepped { attempt, result: Err(error) })
    }

    /// L'accesso è completo (non un ripristino): il core saluta e legge gli amici.
    pub fn signed_in(&mut self, session: crate::auth::Session, me: crate::api::Me) -> Option<Arc<Snapshot>> {
        let result = Ok(Box::new((session, crate::auth::ClerkUser::default(), me)));
        self.social_done(social::Done::Entered { restored: false, result })
    }

    /// Gli inviti alle jam sono arrivati (o no).
    pub fn invites_loaded(&mut self, result: Result<Vec<JamInvite>, AppError>) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::Invites(result))
    }

    /// Il profilo si è salvato (o no).
    pub fn profile_saved(&mut self, result: Result<MyProfile, AppError>) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::ProfileSaved(result))
    }

    /// Un invio che non mostra nulla (brano in ascolto, ascolto, playlist) è finito.
    pub fn social_synced(&mut self, result: Result<(), AppError>) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::Sync(result))
    }

    /// Il profilo di un utente è arrivato (o no).
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

    /// La ricerca di persone ha risposto.
    pub fn users_found(&mut self, result: Result<Vec<Profile>, AppError>) -> Option<Arc<Snapshot>> {
        let ticket = self.state.social.found_ticket();
        self.social_done(social::Done::Found { ticket, result })
    }

    /// Un'azione d'amicizia è finita.
    pub fn friend_changed(
        &mut self,
        handle: &str,
        result: Result<Relation, AppError>,
    ) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::FriendChanged { handle: handle.to_owned(), result })
    }

    /// La playlist pubblica di un amico è arrivata (o no).
    pub fn friend_playlist_loaded(
        &mut self,
        result: Result<Arc<PublicPlaylist>, AppError>,
    ) -> Option<Arc<Snapshot>> {
        let ticket = self.state.social.playlist_ticket();
        self.social_done(social::Done::PlaylistLoaded { ticket, result })
    }

    /// Gli inviti alla jam sono partiti (o no).
    pub fn invited(&mut self, result: Result<usize, AppError>) -> Option<Arc<Snapshot>> {
        self.social_done(social::Done::Invited(result))
    }

    /// Un comando dal sistema (un tasto multimediale), come l'avrebbe dato l'host.
    pub fn media(&mut self, event: media::Event) -> Option<Arc<Snapshot>> {
        self.run(Input::Media(event));
        self.publish()
    }

    /// Un evento della stanza della jam, come l'avrebbe dato il collegamento all'host.
    pub fn jam_event(&mut self, event: crate::jam::Event) -> Option<Arc<Snapshot>> {
        self.run(Input::Jam(event));
        self.publish()
    }

    /// I brani chiesti al catalogo per la coda condivisa sono arrivati (o no).
    pub fn jam_tracks_loaded(&mut self, result: Result<Vec<Track>, AppError>) -> Option<Arc<Snapshot>> {
        let ticket = self.state.jam.fetch_ticket()?;
        self.run(Input::Done(Done::Jam(jam::Done::Tracks { ticket, result })));
        self.publish()
    }

    /// Ciò che la jam ha chiesto al collegamento con la stanza dall'ultima volta (avviarlo, mandare
    /// lo stato locale, chiuderlo): nel core vero lo esegue l'host.
    pub fn take_jam(&mut self) -> Vec<JamOp> {
        std::mem::take(&mut self.jam)
    }

    /// Ciò che il core ha chiesto di mostrare al sistema o a Discord dall'ultima volta.
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

    /// Dà l'ingresso al riduttore ed esegue ciò che chiede, l'audio compreso: ciò che l'audio
    /// risponde rientra subito, come nell'host.
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
                    // Niente scrive sul disco, e nessun altro timer serve ai test dei frontend.
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
